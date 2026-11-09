//! The boolean corpus (steps 17.2 – 17.3): hard cases. Every result
//! of closed inputs must be closed, and volumes must agree with the inputs'
//! (`V(A ∪ B) + V(A ∩ B) = V(A) + V(B)`, `V(A − B) = V(A) − V(A ∩ B)`) or with
//! the expected value.

#![allow(clippy::unwrap_used, reason = "test code")]

use std::collections::{HashMap, HashSet};

use glam::{Quat, Vec2, Vec3};

use super::*;
use crate::primitives;

fn moved(mut mesh: PolyMesh, by: Vec3) -> PolyMesh {
    for p in mesh.points_mut() {
        *p += by;
    }
    mesh
}

fn turned(mut mesh: PolyMesh, rotation: Quat) -> PolyMesh {
    for p in mesh.points_mut() {
        *p = rotation * *p;
    }
    mesh
}

fn cube(size: f32) -> PolyMesh {
    primitives::cube(Vec3::splat(size), [1, 1, 1])
}

fn joined(meshes: &[PolyMesh]) -> PolyMesh {
    let mut out = PolyMesh::new();
    for mesh in meshes {
        let base = u32_index(out.point_count());
        for &p in mesh.points() {
            out.add_point(p).unwrap();
        }
        for face in mesh.faces() {
            let face: Vec<u32> = face.iter().map(|&c| c + base).collect();
            out.add_face(&face).unwrap();
        }
    }
    out
}

/// The edges used more often one way than the other, by their points.
fn open_edges(mesh: &PolyMesh) -> Vec<(Vec3, Vec3, i32)> {
    let mut balance: HashMap<(u32, u32), i32> = HashMap::new();
    for face in mesh.faces() {
        for (i, &u) in face.iter().enumerate() {
            let v = face[(i + 1) % face.len()];
            *balance.entry((u.min(v), u.max(v))).or_default() += if u < v { 1 } else { -1 };
        }
    }
    let mut open: Vec<_> = balance
        .into_iter()
        .filter(|&(_, b)| b != 0)
        .map(|((u, v), b)| (mesh.points()[us(u)], mesh.points()[us(v)], b))
        .collect();
    open.sort_by(|a, b| a.0.x.total_cmp(&b.0.x));
    open.truncate(12);
    open
}

/// Exact results: no weld, so nothing is decided by a distance.
const EXACT: BooleanOptions = BooleanOptions {
    merge_faces: true,
    weld: 0.0,
};

/// Every mode of `a` and `b`: the results' volumes, each checked closed
/// (Without: open, from A only). `None` when a result was not closed.
fn run(a: &PolyMesh, b: &PolyMesh, options: BooleanOptions) -> Result<[f64; 3], String> {
    let mut volumes = [0.0; 3];
    for (slot, mode) in [BooleanMode::Union, BooleanMode::Subtract, BooleanMode::Intersect]
        .into_iter()
        .enumerate()
    {
        let result = boolean(a, b, mode, options).map_err(|e| format!("{mode:?}: {e:?}"))?;
        if !is_closed(&result.mesh) {
            return Err(format!(
                "{mode:?} is not closed ({} faces); open edges: {:?}",
                result.mesh.face_count(),
                open_edges(&result.mesh)
            ));
        }
        assert_eq!(result.origins.len(), result.mesh.face_count());
        volumes[slot] = volume(&result.mesh);
    }
    let without = boolean(a, b, BooleanMode::Without, options).map_err(|e| format!("{e:?}"))?;
    assert!(without.origins.iter().all(|o| o.input == Input::A));
    Ok(volumes)
}

/// Volumes agree to 10⁻⁵ of the inputs' exactly, to 2 % once welded (a
/// weld moves points by up to its distance).
fn close(a: f64, b: f64, scale: f64, options: BooleanOptions) -> bool {
    let tolerance = if options.weld > 0.0 { 2e-2 } else { 1e-5 };
    (a - b).abs() <= tolerance * scale.max(1.0)
}

/// Checks the identities with `options`; `expected` = (union, subtract,
/// intersect) when known.
fn check(
    name: &str,
    a: &PolyMesh,
    b: &PolyMesh,
    options: BooleanOptions,
    expected: Option<[f64; 3]>,
) -> Result<(), String> {
    let [union, subtract, intersect] = run(a, b, options).map_err(|e| format!("{name}: {e}"))?;
    let (va, vb) = (volume(a), volume(b));
    let scale = va.abs() + vb.abs();
    if !close(union + intersect, va + vb, scale, options) || !close(subtract, va - intersect, scale, options) {
        return Err(format!(
            "{name}: volumes disagree: union {union}, subtract {subtract}, intersect {intersect}, A {va}, B {vb}"
        ));
    }
    if let Some([u, s, i]) = expected
        && !(close(union, u, scale, options)
            && close(subtract, s, scale, options)
            && close(intersect, i, scale, options))
    {
        return Err(format!(
            "{name}: expected {u}, {s}, {i}; got {union}, {subtract}, {intersect}"
        ));
    }
    Ok(())
}

/// The corpus: (name, A, B, expected volumes when known).
fn corpus() -> Vec<(&'static str, PolyMesh, PolyMesh, Option<[f64; 3]>)> {
    let a = cube(2.0);
    let i = 1.5 * 1.3 * 1.7;
    vec![
        (
            "overlapping cubes",
            a.clone(),
            moved(cube(2.0), Vec3::new(0.5, 0.7, 0.3)),
            Some([16.0 - i, 8.0 - i, i]),
        ),
        (
            "coplanar faces",
            a.clone(),
            moved(cube(2.0), Vec3::new(1.0, 0.0, 0.0)),
            Some([12.0, 4.0, 4.0]),
        ),
        (
            "a shared edge",
            a.clone(),
            moved(cube(2.0), Vec3::new(2.0, 2.0, 0.0)),
            Some([16.0, 8.0, 0.0]),
        ),
        (
            "a shared point",
            a.clone(),
            moved(cube(2.0), Vec3::new(2.0, 2.0, 2.0)),
            Some([16.0, 8.0, 0.0]),
        ),
        (
            "face to face",
            a.clone(),
            moved(cube(2.0), Vec3::new(2.0, 0.0, 0.0)),
            Some([16.0, 8.0, 0.0]),
        ),
        ("identical", a.clone(), cube(2.0), Some([8.0, 0.0, 8.0])),
        (
            "nearly touching",
            a.clone(),
            moved(cube(2.0), Vec3::new(2.0 + 1e-5, 0.3, 0.1)),
            Some([16.0, 8.0, 0.0]),
        ),
        (
            "a hole flush with two faces",
            a.clone(),
            moved(
                primitives::cube(Vec3::new(1.0, 1.0, 2.0), [1, 1, 1]),
                Vec3::new(0.5, 0.5, 0.0),
            ),
            Some([8.0, 6.0, 2.0]),
        ),
        (
            "a cube turned by a hair",
            a.clone(),
            moved(turned(cube(2.0), Quat::from_rotation_z(1e-6)), Vec3::new(1.0, 0.0, 0.0)),
            None,
        ),
        (
            "a cube inside, touching from within",
            a,
            moved(cube(1.0), Vec3::new(0.5, 0.0, 0.0)),
            Some([8.0, 7.0, 1.0]),
        ),
        (
            "spheres",
            primitives::sphere(1.0, 24),
            moved(primitives::sphere(0.8, 24), Vec3::new(0.7, 0.3, 0.2)),
            None,
        ),
        (
            "a sphere and a cylinder sharing its centre",
            primitives::sphere(1.0, 16),
            primitives::cylinder(0.4, 3.0, 12, 1),
            None,
        ),
        (
            "a plate and nine cylinders",
            primitives::cube(Vec3::new(6.0, 0.5, 6.0), [1, 1, 1]),
            joined(
                &(0..9)
                    .map(|k| {
                        let (x, z) = ((k % 3) as f32 * 2.0 - 2.0, (k / 3) as f32 * 2.0 - 2.0);
                        moved(primitives::cylinder(0.5, 2.0, 16, 1), Vec3::new(x, 0.0, z))
                    })
                    .collect::<Vec<_>>(),
            ),
            None,
        ),
        (
            "two cubes sharing an edge, cut across",
            joined(&[cube(2.0), moved(cube(2.0), Vec3::new(2.0, 2.0, 0.0))]),
            moved(
                primitives::cube(Vec3::new(1.0, 6.0, 1.0), [1, 1, 1]),
                Vec3::new(1.0, 1.0, 0.0),
            ),
            None,
        ),
    ]
}

/// `mesh` with points at equal coordinates made one (primitives keep their
/// caps' points apart).
fn welded(mesh: &PolyMesh) -> PolyMesh {
    let mut index = HashMap::new();
    let mut points = Vec::new();
    let faces: Vec<Vec<u32>> = mesh
        .faces()
        .map(|face| {
            face.iter()
                .map(|&c| {
                    let p = mesh.points()[us(c)];
                    *index.entry(p.to_array().map(f32::to_bits)).or_insert_with(|| {
                        points.push(p);
                        u32_index(points.len() - 1)
                    })
                })
                .collect()
        })
        .collect();
    PolyMesh::from_faces(points, faces).unwrap()
}

#[test]
fn the_inputs_are_closed() {
    for (name, a, b, _) in corpus() {
        assert!(is_closed(&welded(&a)) && is_closed(&welded(&b)), "{name}");
        assert!(volume(&a) > 0.0 && volume(&b) > 0.0, "{name} faces outwards");
    }
}

#[test]
fn the_exact_method_passes_the_whole_corpus() {
    let failures: Vec<String> = corpus()
        .into_iter()
        .filter_map(|(name, a, b, expected)| check(name, &a, &b, EXACT, expected).err())
        .collect();
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn the_corpus_passes_welded_too() {
    let failures: Vec<String> = corpus()
        .into_iter()
        .filter_map(|(name, a, b, expected)| check(name, &a, &b, BooleanOptions::default(), expected).err())
        .collect();
    assert!(failures.is_empty(), "{failures:#?}");
}

/// A cube with a corner cut away: what is left of each face is one polygon.
#[test]
fn cut_faces_are_merged_back_into_polygons() {
    let a = cube(2.0);
    let b = moved(cube(2.0), Vec3::splat(1.0));
    let result = boolean(&a, &b, BooleanMode::Subtract, EXACT).unwrap();
    assert!(is_closed(&result.mesh));
    assert!((volume(&result.mesh) - 7.0).abs() < 1e-6);
    assert_eq!(result.mesh.face_count(), 9, "6 of A (3 cut, 3 whole), 3 of B");
    let from_b = result.origins.iter().filter(|o| o.input == Input::B).count();
    assert_eq!(from_b, 3);
    // Merging off: only triangles.
    let options = BooleanOptions {
        merge_faces: false,
        ..EXACT
    };
    let triangles = boolean(&a, &b, BooleanMode::Subtract, options).unwrap();
    assert!(is_closed(&triangles.mesh));
    let cut_faces = triangles.mesh.faces().zip(&triangles.origins);
    for (face, origin) in cut_faces {
        let whole = origin.input == Input::A && a.face_center(us(origin.face)).min_element() < -0.5;
        assert!(whole || face.len() == 3, "{origin:?} has {} corners", face.len());
    }
}

#[test]
fn uvs_follow_the_faces() {
    let a = cube(2.0);
    assert!(a.uvs().is_some(), "primitives have UVs");
    let b = moved(cube(2.0), Vec3::new(1.0, 0.6, 0.3));
    let result = boolean(&a, &b, BooleanMode::Subtract, EXACT).unwrap();
    let uvs = result.mesh.uvs().unwrap();
    let mut corner = 0;
    for (face, origin) in result.mesh.faces().zip(&result.origins) {
        let source = if origin.input == Input::A { &a } else { &b };
        let range = source.corner_range(us(origin.face));
        for &p in face {
            let uv = uvs[corner];
            corner += 1;
            assert!(uv.is_finite() && uv.cmpge(Vec2::splat(-1e-4)).all() && uv.cmple(Vec2::splat(1.0001)).all());
            // At an input point, the input's own UV.
            let position = result.mesh.points()[us(p)];
            for c in range.clone() {
                if source.points()[us(source.corner_points()[c])] == position {
                    // B's faces are seen from inside: their pictures mirrored back.
                    let mut expected = source.uvs().unwrap()[c];
                    if origin.input == Input::B {
                        expected.x = 1.0 - expected.x;
                    }
                    assert!((expected - uv).length() < 1e-5, "{origin:?}");
                }
            }
        }
    }
}

#[test]
fn the_cut_edges_are_listed() {
    let a = cube(2.0);
    let b = moved(cube(2.0), Vec3::splat(1.0));
    for options in [EXACT, BooleanOptions::default()] {
        let result = boolean(&a, &b, BooleanMode::Subtract, options).unwrap();
        let length: f32 = result
            .cut_edges
            .iter()
            .map(|&[u, v]| result.mesh.points()[us(u)].distance(result.mesh.points()[us(v)]))
            .sum();
        assert!(
            (length - 6.0).abs() < 1e-5,
            "two unit edges on each of A's three faces: {length}"
        );
        let edges: HashSet<(u32, u32)> = result
            .mesh
            .faces()
            .flat_map(|f| (0..f.len()).map(move |i| (f[i].min(f[(i + 1) % f.len()]), f[i].max(f[(i + 1) % f.len()]))))
            .collect();
        assert!(result.cut_edges.iter().all(|&[u, v]| edges.contains(&(u, v))));
    }
}

#[test]
fn a_weld_joins_what_is_closer_than_it() {
    // B's face 0.001 inside A's: the sliver between them is welded away.
    let a = cube(2.0);
    let b = moved(cube(2.0), Vec3::new(1.999, 0.5, 0.25));
    let exact = boolean(&a, &b, BooleanMode::Union, EXACT).unwrap();
    let welded = boolean(&a, &b, BooleanMode::Union, BooleanOptions::default()).unwrap();
    assert!(is_closed(&welded.mesh));
    let shortest = |mesh: &PolyMesh| {
        mesh.faces()
            .flat_map(|f| (0..f.len()).map(move |i| (f[i], f[(i + 1) % f.len()])))
            .map(|(u, v)| mesh.points()[us(u)].distance(mesh.points()[us(v)]))
            .fold(f32::INFINITY, f32::min)
    };
    assert!(shortest(&exact.mesh) < 0.01);
    assert!(shortest(&welded.mesh) >= 0.01, "{}", shortest(&welded.mesh));
}

#[test]
fn an_open_cutter_still_cuts() {
    // B lacks the face that lies outside A: the winding number still says
    // what is inside it, and the cut A stays closed.
    let a = cube(2.0);
    let full = moved(cube(2.0), Vec3::new(1.5, 0.3, 0.2));
    let open = full.with_faces_where(|f| full.face_center(f).x < 2.4);
    assert!(!is_closed(&open));
    {
        let result = boolean(&a, &open, BooleanMode::Subtract, EXACT).unwrap();
        assert!(is_closed(&result.mesh));
        let expected = 8.0 - 0.5 * 1.7 * 1.8;
        assert!((volume(&result.mesh) - expected).abs() < 1e-4);
    }
}

#[test]
fn faces_remember_their_input_and_face() {
    let a = cube(2.0);
    let b = moved(cube(2.0), Vec3::new(1.0, 1.0, 1.0));
    let result = boolean(&a, &b, BooleanMode::Subtract, EXACT).unwrap();
    for (face, origin) in result.mesh.faces().zip(&result.origins) {
        let source = if origin.input == Input::A { &a } else { &b };
        let normal = source.face_normal(us(origin.face));
        let points: Vec<Vec3> = face.iter().map(|&c| result.mesh.points()[us(c)]).collect();
        let own = (points[1] - points[0]).cross(points[2] - points[0]).normalize();
        let expected = if origin.input == Input::A { normal } else { -normal };
        assert!(own.dot(expected) > 0.999, "{origin:?}");
    }
    assert!(result.origins.iter().any(|o| o.input == Input::B));
}

#[test]
fn bad_and_empty_inputs() {
    let a = cube(2.0);
    let mut bad = cube(1.0);
    bad.points_mut()[0].x = f32::NAN;
    assert_eq!(
        boolean(&a, &bad, BooleanMode::Union, EXACT).unwrap_err(),
        BooleanError::NotFinite
    );
    let empty = PolyMesh::new();
    let union = boolean(&a, &empty, BooleanMode::Union, EXACT).unwrap();
    assert!((volume(&union.mesh) - 8.0).abs() < 1e-6);
    let none = boolean(&empty, &a, BooleanMode::Subtract, EXACT).unwrap();
    assert_eq!(none.mesh.face_count(), 0);
}

mod random {
    //! Random shapes, placed on a coarse grid half the time so faces lie in
    //! one plane, edges and points meet and shapes touch (the fuzzed part of
    //! the corpus).

    use proptest::prelude::*;

    use super::*;

    fn shape() -> impl Strategy<Value = PolyMesh> {
        let place = prop_oneof![
            (-8i32..=8, -8i32..=8, -8i32..=8).prop_map(|(x, y, z)| Vec3::new(x as f32, y as f32, z as f32) * 0.25),
            (-2.0f32..2.0, -2.0f32..2.0, -2.0f32..2.0).prop_map(|(x, y, z)| Vec3::new(x, y, z)),
        ];
        let turn = prop_oneof![
            (0u8..4).prop_map(|k| Quat::from_rotation_y(f32::from(k) * std::f32::consts::FRAC_PI_2)),
            (0.0f32..6.3, 0.0f32..6.3).prop_map(|(a, b)| Quat::from_rotation_y(a) * Quat::from_rotation_x(b)),
        ];
        let body = prop_oneof![
            (1u8..=8).prop_map(|s| cube(f32::from(s) * 0.5)),
            (1u8..=6, 1u8..=6, 1u8..=6).prop_map(|(x, y, z)| primitives::cube(
                Vec3::new(f32::from(x), f32::from(y), f32::from(z)) * 0.5,
                [1, 1, 1]
            )),
            (0.3f32..1.5, 6u32..12).prop_map(|(r, s)| primitives::sphere(r, s)),
            (0.2f32..1.0, 0.5f32..3.0, 5u32..10).prop_map(|(r, h, s)| primitives::cylinder(r, h, s, 1)),
        ];
        (body, turn, place).prop_map(|(body, turn, place)| moved(turned(body, turn), place))
    }

    /// Points on a coarse grid (so many coincide or line up) and faces
    /// between any of them (invalid ones skipped).
    fn garbage() -> impl Strategy<Value = PolyMesh> {
        (
            prop::collection::vec((-4i8..=4, -4i8..=4, -4i8..=4), 3..16),
            prop::collection::vec(prop::collection::vec(0u32..16, 3..7), 0..16),
        )
            .prop_map(|(points, faces)| {
                let mut mesh = PolyMesh::new();
                for (x, y, z) in &points {
                    mesh.add_point(Vec3::new(f32::from(*x), f32::from(*y), f32::from(*z)) * 0.5)
                        .unwrap();
                }
                let count = u32_index(points.len());
                for face in faces {
                    let face: Vec<u32> = face.into_iter().map(|c| c % count).collect();
                    let _ = mesh.add_face(&face);
                }
                mesh
            })
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

        /// Any points and faces (open, self-crossing, folded, flat): a
        /// result or an error, never a panic.
        #[test]
        fn any_mesh_gives_a_result_without_panicking(
            a in garbage(),
            b in garbage(),
            mode in prop::sample::select(BooleanMode::ALL.to_vec()),
        ) {
            prop_assert!(boolean(&a, &b, mode, EXACT).is_ok());
            prop_assert!(boolean(&cube(2.0), &b, mode, EXACT).is_ok());
        }

        /// Closed in, closed out; the volumes agree with the inputs'.
        #[test]
        fn the_exact_method_keeps_random_shapes_closed(a in shape(), b in shape()) {
            let result = check("random", &a, &b, EXACT, None);
            prop_assert!(result.is_ok(), "{:?}", result);
        }
    }
}
