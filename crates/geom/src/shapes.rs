//! Hand-built meshes for this crate's tests. Every face is counter-clockwise
//! when seen from outside. (The parametric primitives of the editor arrive with
//! step 3.2 and get their own tests.)

use std::f32::consts::TAU;

use glam::Vec3;

use crate::PolyMesh;

fn angle(i: u32, n: u32) -> f32 {
    TAU * f32::from(u16::try_from(i).unwrap()) / f32::from(u16::try_from(n).unwrap())
}

/// Axis-aligned cube of edge `size` centred on the origin: 8 points, 6 quads.
/// Point `i` sits at `(±, ±, ±)` by its bits (x = bit 0, y = bit 1, z = bit 2).
pub(crate) fn cube(size: f32) -> PolyMesh {
    let h = size * 0.5;
    let points = (0..8u8)
        .map(|i| {
            Vec3::new(
                if i & 1 == 0 { -h } else { h },
                if i & 2 == 0 { -h } else { h },
                if i & 4 == 0 { -h } else { h },
            )
        })
        .collect();
    // For normal n, with u × v = n: (−u−v, +u−v, +u+v, −u+v) is counter-clockwise.
    PolyMesh::from_faces(
        points,
        [
            [1u32, 3, 7, 5], // +X
            [0, 4, 6, 2],    // −X
            [2, 6, 7, 3],    // +Y
            [0, 1, 5, 4],    // −Y
            [4, 5, 7, 6],    // +Z
            [0, 2, 3, 1],    // −Z
        ],
    )
    .unwrap()
}

/// The unit cube without its top (+Y) face.
pub(crate) fn open_box() -> PolyMesh {
    let cube = cube(1.0);
    let faces: Vec<&[u32]> = cube
        .faces()
        .enumerate()
        .filter(|&(f, _)| f != 2)
        .map(|(_, c)| c)
        .collect();
    PolyMesh::from_faces(cube.points().to_vec(), faces).unwrap()
}

/// `nx × nz` unit quads on the XZ plane, facing +Y.
pub(crate) fn grid(nx: u32, nz: u32) -> PolyMesh {
    let mut points = Vec::new();
    for j in 0..=nz {
        for i in 0..=nx {
            points.push(Vec3::new(
                f32::from(u16::try_from(i).unwrap()),
                0.0,
                f32::from(u16::try_from(j).unwrap()),
            ));
        }
    }
    let p = |i: u32, j: u32| j * (nx + 1) + i;
    let mut faces = Vec::new();
    for j in 0..nz {
        for i in 0..nx {
            // +Z then +X: Z × X = +Y.
            faces.push([p(i, j), p(i, j + 1), p(i + 1, j + 1), p(i + 1, j)]);
        }
    }
    PolyMesh::from_faces(points, faces).unwrap()
}

/// Capped cylinder (radius 1, height 1, axis +Y): `segments` side quads and two
/// `segments`-gon caps.
pub(crate) fn cylinder(segments: u32) -> PolyMesh {
    let ring = |y: f32| (0..segments).map(move |i| Vec3::new(angle(i, segments).sin(), y, angle(i, segments).cos()));
    let points: Vec<Vec3> = ring(-0.5).chain(ring(0.5)).collect();
    let (b, t) = (|i: u32| i % segments, |i: u32| segments + i % segments);
    let mut faces: Vec<Vec<u32>> = (0..segments).map(|i| vec![b(i), b(i + 1), t(i + 1), t(i)]).collect();
    faces.push((0..segments).map(t).collect()); // top, +Y
    faces.push((0..segments).rev().map(b).collect()); // bottom, −Y
    PolyMesh::from_faces(points, faces).unwrap()
}

/// Torus around +Y (major radius 2, tube radius 0.5) of `major × minor` quads.
pub(crate) fn torus(major: u32, minor: u32) -> PolyMesh {
    let (big, small) = (2.0, 0.5);
    let mut points = Vec::new();
    for i in 0..major {
        let u = angle(i, major);
        for j in 0..minor {
            let v = angle(j, minor);
            let r = big + small * v.cos();
            points.push(Vec3::new(r * u.cos(), small * v.sin(), r * u.sin()));
        }
    }
    let p = |i: u32, j: u32| (i % major) * minor + j % minor;
    let mut faces = Vec::new();
    for i in 0..major {
        for j in 0..minor {
            // Around the tube (+Y at the outer equator), then around the axis (+Z): outward.
            faces.push([p(i, j), p(i, j + 1), p(i + 1, j + 1), p(i + 1, j)]);
        }
    }
    PolyMesh::from_faces(points, faces).unwrap()
}

/// One non-convex hexagon in the XY plane facing +Z: an L of three unit squares.
pub(crate) fn l_shape() -> PolyMesh {
    let points = [(0.0, 0.0), (2.0, 0.0), (2.0, 1.0), (1.0, 1.0), (1.0, 2.0), (0.0, 2.0)]
        .map(|(x, y)| Vec3::new(x, y, 0.0))
        .to_vec();
    PolyMesh::from_faces(points, [[0u32, 1, 2, 3, 4, 5]]).unwrap()
}

/// One five-pointed star (a 10-gon, very non-convex) in the XY plane facing +Z.
pub(crate) fn star() -> PolyMesh {
    let points = (0..10u32)
        .map(|i| {
            let r = if i % 2 == 0 { 1.0 } else { 0.38 };
            let a = angle(i, 10);
            Vec3::new(r * a.cos(), r * a.sin(), 0.0)
        })
        .collect();
    PolyMesh::from_faces(points, [(0..10u32).collect::<Vec<_>>()]).unwrap()
}

/// A zero-area pentagon: five points on a line.
pub(crate) fn collinear() -> PolyMesh {
    let points = (0..5u8).map(|i| Vec3::new(f32::from(i), 0.0, 0.0)).collect();
    PolyMesh::from_faces(points, [[0u32, 1, 2, 3, 4]]).unwrap()
}

/// Every closed test shape (for invariants that hold on all of them).
pub(crate) fn closed() -> Vec<(&'static str, PolyMesh)> {
    vec![("cube", cube(2.0)), ("cylinder", cylinder(12)), ("torus", torus(16, 8))]
}
