//! Bridge: a band of polygons between two polygons or two border edges.

use super::{Builder, Corner, Rebuilt, corner_faces, next_corner};
use crate::{MeshError, PolyMesh, Topology, us};

/// Bridge options (see `docs/reference/modeling-tools.md`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BridgeOptions {
    /// Rows of polygons along the bridge.
    pub subdivision: u32,
    /// Turns which point of the second element each point of the first joins.
    pub spin: i32,
    /// Remove the two bridged polygons, so the bridge opens through them.
    pub delete_originals: bool,
}

/// Joins faces `a` and `b` (same number of points, no shared point) by a band of
/// quads. Their points pair up in opposite order, so the band continues both
/// faces' neighbours; the pairing with the shortest total distance is chosen,
/// then turned by `spin`. Unchanged if the faces cannot be bridged.
pub fn bridge_polygons(mesh: &PolyMesh, a: usize, b: usize, options: &BridgeOptions) -> Result<Rebuilt, MeshError> {
    let mut builder = Builder::new(mesh);
    let (fa, fb) = (mesh.face(a), mesh.face(b));
    let n = fa.len();
    let bridgeable = a != b && n >= 3 && n == fb.len() && !fa.iter().any(|p| fb.contains(p));
    if !bridgeable {
        return keep_all(builder, mesh);
    }
    let pos = |p: u32| mesh.points()[us(p)];
    // Point i of `a` joins point (shift - i) of `b`.
    let partner = |shift: usize, i: usize| fb[(shift + n - i % n) % n];
    let cost = |shift: usize| {
        (0..n)
            .map(|i| pos(fa[i]).distance_squared(pos(partner(shift, i))))
            .sum::<f32>()
    };
    let best = (0..n).min_by(|&x, &y| cost(x).total_cmp(&cost(y))).unwrap_or(0);
    let spin = usize::try_from(options.spin.rem_euclid(i32::try_from(n).unwrap_or(1))).unwrap_or(0);
    let shift = (best + spin) % n;

    for f in 0..mesh.face_count() {
        if options.delete_originals && (f == a || f == b) {
            continue;
        }
        builder.keep(f);
    }
    // Rows of points from each point of `a` to its partner.
    let steps = options.subdivision + 1;
    let columns: Vec<Vec<u32>> = (0..n)
        .map(|i| {
            let (from, to) = (fa[i], partner(shift, i));
            let mut column = vec![from];
            for k in 1..steps {
                let t = fraction(k, steps);
                column.push(builder.add_point(pos(from).lerp(pos(to), t)));
            }
            column.push(to);
            column
        })
        .collect();
    let corners_of = |points: [u32; 4]| points.map(|p| Corner::Point(p, glam::Vec2::ZERO));
    for i in 0..n {
        let (left, right) = (&columns[i], &columns[(i + 1) % n]);
        for k in 0..left.len() - 1 {
            let mut quad = corners_of([left[k], right[k], right[k + 1], left[k + 1]]);
            // Replacing the two faces, the band runs their edges their way; next to
            // them (kept), it runs them the other way.
            if !options.delete_originals {
                quad.reverse();
            }
            builder.push_new(&quad);
        }
    }
    builder.finish()
}

/// Joins border edges `a` and `b` (unique edge indices) by one polygon (a row of
/// `subdivision + 1` when they share no point) that continues both neighbours'
/// winding. Unchanged unless both are border edges.
pub fn bridge_edges(
    mesh: &PolyMesh,
    topology: &Topology,
    a: usize,
    b: usize,
    options: &BridgeOptions,
) -> Result<Rebuilt, MeshError> {
    let mut builder = Builder::new(mesh);
    let corner_faces = corner_faces(mesh);
    // The edge as its face runs it.
    let run = |e: usize| {
        let &[c] = topology.edge_corners(e) else { return None };
        let c = us(c);
        let next = next_corner(mesh, us(corner_faces[c]), c);
        Some((mesh.corner_points()[c], mesh.corner_points()[next]))
    };
    let (Some((p, q)), Some((r, s))) = (run(a), run(b)) else {
        return keep_all(builder, mesh);
    };
    // The new polygon runs q → p and s → r: the opposite of both neighbours.
    let mut ring = vec![q, p, s, r];
    ring.dedup();
    if ring.len() > 1 && ring.first() == ring.last() {
        ring.pop();
    }
    let mut distinct = ring.clone();
    distinct.sort_unstable();
    distinct.dedup();
    if a == b || distinct.len() != ring.len() || ring.len() < 3 {
        return keep_all(builder, mesh);
    }
    for f in 0..mesh.face_count() {
        builder.keep(f);
    }
    if ring.len() == 3 {
        // The edges share a point: one triangle.
        let corners: Vec<Corner> = ring.iter().map(|&x| Corner::Point(x, glam::Vec2::ZERO)).collect();
        builder.push_new(&corners);
        return builder.finish();
    }
    let pos = |x: u32| mesh.points()[us(x)];
    let steps = options.subdivision + 1;
    let column = |builder: &mut Builder<'_>, from: u32, to: u32| {
        let mut points = vec![from];
        for k in 1..steps {
            points.push(builder.add_point(pos(from).lerp(pos(to), fraction(k, steps))));
        }
        points.push(to);
        points
    };
    let (left, right) = (column(&mut builder, p, s), column(&mut builder, q, r));
    for k in 0..left.len() - 1 {
        let quad = [right[k], left[k], left[k + 1], right[k + 1]].map(|x| Corner::Point(x, glam::Vec2::ZERO));
        builder.push_new(&quad);
    }
    builder.finish()
}

fn fraction(k: u32, n: u32) -> f32 {
    let f = |v: u32| f32::from(u16::try_from(v).unwrap_or(u16::MAX));
    f(k) / f(n)
}

fn keep_all(mut builder: Builder<'_>, mesh: &PolyMesh) -> Result<Rebuilt, MeshError> {
    for f in 0..mesh.face_count() {
        builder.keep(f);
    }
    builder.finish()
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::*;
    use crate::shapes;

    const OPTIONS: BridgeOptions = BridgeOptions {
        subdivision: 0,
        spin: 0,
        delete_originals: true,
    };

    fn face_with_normal(mesh: &PolyMesh, normal: Vec3) -> usize {
        (0..mesh.face_count())
            .find(|&f| mesh.face_normal(f).abs_diff_eq(normal, 1e-5))
            .unwrap()
    }

    /// The face with `normal` whose centre is on the vertical axis.
    fn centre_face(mesh: &PolyMesh, normal: Vec3) -> usize {
        (0..mesh.face_count())
            .find(|&f| {
                let c = mesh.face_center(f);
                mesh.face_normal(f).abs_diff_eq(normal, 1e-5) && c.x.abs() < 1e-4 && c.z.abs() < 1e-4
            })
            .unwrap()
    }

    #[test]
    fn bridging_the_middles_of_top_and_bottom_makes_a_hole_through() {
        let cube = crate::primitives::cube(Vec3::splat(3.0), [3, 3, 3]);
        let (top, bottom) = (centre_face(&cube, Vec3::Y), centre_face(&cube, -Vec3::Y));
        let out = bridge_polygons(&cube, top, bottom, &OPTIONS).unwrap();
        assert_eq!(out.mesh.face_count(), cube.face_count() - 2 + 4);
        let topology = Topology::build(&out.mesh);
        assert!(topology.is_manifold() && topology.is_closed());
        assert_eq!(topology.euler_characteristic(), 0, "a ring: genus 1");
        assert_eq!(out.face_origin.iter().filter(|&&o| o == super::super::NEW).count(), 4);
        // The walls run straight down: each joins points above each other.
        for wall in out.mesh.faces().skip(cube.face_count() - 2) {
            let xz = |p: u32| out.mesh.points()[us(p)] * Vec3::new(1.0, 0.0, 1.0);
            assert!(xz(wall[0]).abs_diff_eq(xz(wall[3]), 1e-5), "untwisted");
        }
        let rows = bridge_polygons(
            &cube,
            top,
            bottom,
            &BridgeOptions {
                subdivision: 1,
                ..OPTIONS
            },
        )
        .unwrap();
        assert_eq!(rows.mesh.face_count(), cube.face_count() - 2 + 8);
        assert!(Topology::build(&rows.mesh).is_closed());
        let twisted = bridge_polygons(&cube, top, bottom, &BridgeOptions { spin: 1, ..OPTIONS }).unwrap();
        assert!(Topology::build(&twisted.mesh).is_manifold());
        assert_ne!(twisted.mesh, out.mesh);
    }

    #[test]
    fn two_facing_quads_become_a_box() {
        // A quad facing down at y = 0 and one facing up at y = 1.
        let points = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 1.0),
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(1.0, 1.0, 0.0),
            Vec3::new(1.0, 1.0, 1.0),
            Vec3::new(0.0, 1.0, 1.0),
        ];
        let mesh = PolyMesh::from_faces(points, [[0u32, 1, 2, 3], [4, 7, 6, 5]]).unwrap();
        let kept = bridge_polygons(
            &mesh,
            0,
            1,
            &BridgeOptions {
                delete_originals: false,
                ..OPTIONS
            },
        )
        .unwrap();
        assert_eq!(kept.mesh.face_count(), 6);
        let topology = Topology::build(&kept.mesh);
        assert!(topology.is_manifold() && topology.is_closed(), "a closed box");
        let open = bridge_polygons(&mesh, 0, 1, &OPTIONS).unwrap();
        assert_eq!(open.mesh.face_count(), 4, "a tube without its ends");
    }

    #[test]
    fn faces_that_cannot_be_bridged_are_left_alone() {
        let cube = shapes::cube(2.0);
        let (top, side) = (face_with_normal(&cube, Vec3::Y), face_with_normal(&cube, Vec3::X));
        assert_eq!(
            bridge_polygons(&cube, top, side, &OPTIONS).unwrap().mesh,
            cube,
            "they share points"
        );
        let cylinder = shapes::cylinder(6);
        assert_eq!(
            bridge_polygons(&cylinder, 0, 6, &OPTIONS).unwrap().mesh,
            cylinder,
            "a quad and a hexagon"
        );
    }

    #[test]
    fn two_border_edges_get_a_polygon_between_them() {
        // Two separate unit quads, one at x 0..1 and one at x 2..3.
        let points = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(1.0, 0.0, 1.0),
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(3.0, 0.0, 0.0),
            Vec3::new(2.0, 0.0, 1.0),
            Vec3::new(3.0, 0.0, 1.0),
        ];
        let mesh = PolyMesh::from_faces(points, [[0u32, 2, 3, 1], [4, 6, 7, 5]]).unwrap();
        let topology = Topology::build(&mesh);
        let edge = |a: u32, b: u32| {
            topology
                .edges()
                .iter()
                .position(|&e| e == [a.min(b), a.max(b)])
                .unwrap()
        };
        let out = bridge_edges(&mesh, &topology, edge(1, 3), edge(4, 6), &OPTIONS).unwrap();
        assert_eq!(out.mesh.face_count(), 3);
        let result = Topology::build(&out.mesh);
        assert!(result.is_manifold());
        assert_eq!(result.boundary_edge_count(), 8, "one strip of three quads now");
        assert!(out.mesh.face_normal(2).abs_diff_eq(Vec3::Y, 1e-5), "the same winding");
        let rows = bridge_edges(
            &mesh,
            &topology,
            edge(1, 3),
            edge(4, 6),
            &BridgeOptions {
                subdivision: 2,
                ..OPTIONS
            },
        )
        .unwrap();
        assert_eq!((rows.mesh.point_count(), rows.mesh.face_count()), (12, 5));

        // Two edges of one quad that share a point: a triangle.
        let quad = shapes::grid(1, 1);
        let topology = Topology::build(&quad);
        let edge = |a: u32, b: u32| {
            topology
                .edges()
                .iter()
                .position(|&e| e == [a.min(b), a.max(b)])
                .unwrap()
        };
        let out = bridge_edges(&quad, &topology, edge(0, 2), edge(2, 3), &OPTIONS).unwrap();
        assert_eq!(out.mesh.face_count(), 2);
        assert_eq!(out.mesh.face(1).len(), 3);
        // Inner edges cannot be bridged.
        let plane = shapes::grid(2, 1);
        let topology = Topology::build(&plane);
        let inner = topology.edges().iter().position(|&e| e == [1, 4]).unwrap();
        assert_eq!(bridge_edges(&plane, &topology, inner, 0, &OPTIONS).unwrap().mesh, plane);
    }
}
