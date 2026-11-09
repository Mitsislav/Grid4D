//! The modelling budget (Phase 17): what today's modelling costs, measured
//! before the live Boolean, Symmetry and Reduce exist, so their own times
//! have numbers to beat or keep (see `docs/performance.md` ▸ *Modelling*).
//!
//! ```text
//! cargo run --release -p grid4d-eval --example modeling_budget
//! ```
//!
//! Each line is the median of several runs:
//!
//! * **a live generator under a drag** — a Subdivision Surface (two levels)
//!   over an editable sphere cage; each frame moves one cage point (a new
//!   mesh, as the gizmo writes it) and evaluates the scene. This is the cost a
//!   live Boolean or Reduce is weighed against (risk R9);
//! * **what a Boolean starts from** — two 50 000-triangle spheres triangulated
//!   and given a BVH and topology each: a floor under any boolean rebuild;
//! * **what Symmetry does by hand today** — a half of 100 000 points
//!   mirrored, joined to itself and welded at the plane with Optimize (about
//!   200 000 points in the result);
//! * **what Reduce starts from** — the topology and BVH of a
//!   million-triangle scan;
//! * **the Boolean** (step 17.2's spike, both methods of holding the points
//!   a cut makes): a product shell (a box of about 1 700 triangles minus two
//!   cylinders), two spheres of 50 000 triangles, and a sphere of a million
//!   triangles with a cylinder cut through it.

use std::{
    error::Error,
    hint::black_box,
    time::{Duration, Instant},
};

use glam::Vec3;
use grid4d_eval::EvaluatedScene;
use grid4d_geom::{
    BitSet, Bvh, PolyMesh, Smoothing, Topology, TriMesh, Triangulator,
    boolean::{BooleanMode, Method, boolean, is_closed},
    ops::{OptimizeOptions, optimize},
    primitives,
};
use grid4d_scene::{Components, Document, MeshHandle, Node, ObjectKind, ObjectKindTag};

/// Times `run` `count` times and returns the median.
fn median(count: usize, mut run: impl FnMut()) -> Duration {
    let mut times: Vec<Duration> = (0..count)
        .map(|_| {
            let start = Instant::now();
            run();
            start.elapsed()
        })
        .collect();
    times.sort();
    times.get(count / 2).copied().unwrap_or_default()
}

fn ms(duration: Duration) -> String {
    format!("{:8.2} ms", duration.as_secs_f64() * 1000.0)
}

fn polygon(mesh: PolyMesh) -> ObjectKind {
    ObjectKind::Polygon {
        mesh: MeshHandle::new(mesh),
        components: Components::default(),
    }
}

/// A Subdivision Surface at two levels over a sphere cage of `segments`
/// around, one cage point moved per frame.
fn live_generator(segments: u32) -> Result<(usize, Duration), Box<dyn Error>> {
    let mut doc = Document::new("drag");
    let mut kind = ObjectKindTag::SubdivisionSurface.default_kind();
    if let ObjectKind::SubdivisionSurface { viewport_levels, .. } = &mut kind {
        *viewport_levels = 2;
    }
    let sds = doc.scene.insert(Node::new("Subdivision Surface", kind));
    let cage = primitives::sphere(100.0, segments);
    let faces = cage.face_count();
    let cage = doc
        .scene
        .insert_child(sds, Node::new("Cage", polygon(cage)))
        .map_err(|err| format!("{err:?}"))?;
    let mut evaluated = EvaluatedScene::new();
    evaluated.update(&doc.scene);
    let mut step = 0.0f32;
    let time = median(31, || {
        step += 0.1;
        if let Some(node) = doc.scene.get_mut(cage)
            && let ObjectKind::Polygon { mesh, .. } = &mut node.data.kind
        {
            let mut points = mesh.mesh().points().to_vec();
            if let Some(p) = points.first_mut() {
                p.y += step.sin();
            }
            *mesh = mesh.with_points(&points);
        }
        black_box(evaluated.update(&doc.scene));
    });
    Ok((faces, time))
}

/// Triangulating two spheres of about 50 000 triangles and building each one's
/// BVH and topology.
fn boolean_floor() -> (usize, Duration) {
    let a = primitives::sphere(100.0, 224);
    let mut b = primitives::sphere(80.0, 224);
    for p in b.points_mut() {
        *p += Vec3::new(60.0, 20.0, 0.0);
    }
    let mut triangulator = Triangulator::new();
    let mut out = TriMesh::default();
    let mut triangles = 0;
    let time = median(11, || {
        triangles = 0;
        for mesh in [&a, &b] {
            triangulator.triangulate(mesh, Smoothing::default(), &mut out);
            triangles += out.triangle_count();
            black_box(Bvh::build(mesh));
            black_box(Topology::build(mesh));
        }
    });
    (triangles, time)
}

/// A half of about 100 000 points mirrored across x = 0, joined and
/// welded at the plane.
fn symmetry_by_hand() -> Result<(usize, Duration), Box<dyn Error>> {
    // A sphere's half with x ≥ 0: the faces whose centre lies on that side.
    let whole = primitives::sphere(100.0, 448);
    let half = whole.with_faces_where(|f| whole.face_center(f).x >= 0.0);
    let options = OptimizeOptions {
        merge_points: true,
        tolerance: 0.01,
        unused_points: true,
    };
    let mut points = 0;
    let mut failed = None;
    let time = median(11, || {
        let count = u32::try_from(half.point_count()).unwrap_or(u32::MAX);
        let mut joined: Vec<Vec3> = half.points().to_vec();
        joined.extend(half.points().iter().map(|p| Vec3::new(-p.x, p.y, p.z)));
        // The mirrored copy's faces turn the other way round.
        let faces = half
            .faces()
            .map(|f| f.iter().rev().map(|&c| c + count).collect::<Vec<u32>>());
        let faces: Vec<Vec<u32>> = half.faces().map(<[u32]>::to_vec).chain(faces).collect();
        match PolyMesh::from_faces(joined, faces) {
            Ok(mesh) => {
                let mut near = BitSet::new();
                for (p, point) in mesh.points().iter().enumerate() {
                    if point.x.abs() <= options.tolerance {
                        near.insert(p);
                    }
                }
                match optimize(&mesh, Some(&near), &options) {
                    Ok(rebuilt) => points = rebuilt.mesh.point_count(),
                    Err(err) => failed = Some(format!("{err:?}")),
                }
            }
            Err(err) => failed = Some(format!("{err:?}")),
        }
    });
    if let Some(err) = failed {
        return Err(err.into());
    }
    Ok((points, time))
}

/// The topology and BVH of a scan of about a million triangles.
fn reduce_floor() -> (usize, Duration) {
    let mut scan = primitives::sphere(100.0, 1000);
    for p in scan.points_mut() {
        *p *= 1.0 + 0.02 * (p.x * 0.3).sin() * (p.z * 0.2).cos();
    }
    let triangles = scan.faces().map(|f| f.len().saturating_sub(2)).sum::<usize>();
    let time = median(5, || {
        black_box(Topology::build(&scan));
        black_box(Bvh::build(&scan));
    });
    (triangles, time)
}

fn moved(mut mesh: PolyMesh, by: Vec3) -> PolyMesh {
    for p in mesh.points_mut() {
        *p += by;
    }
    mesh
}

fn joined(meshes: &[PolyMesh]) -> Result<PolyMesh, Box<dyn Error>> {
    let mut out = PolyMesh::new();
    for mesh in meshes {
        let base = u32::try_from(out.point_count())?;
        for &p in mesh.points() {
            out.add_point(p).map_err(|e| e.to_string())?;
        }
        for face in mesh.faces() {
            let face: Vec<u32> = face.iter().map(|&c| c + base).collect();
            out.add_face(&face).map_err(|e| e.to_string())?;
        }
    }
    Ok(out)
}

/// A Boolean case: its name, A, B and how many runs to time.
type Case = (&'static str, PolyMesh, PolyMesh, usize);

/// The Boolean's cases.
fn boolean_cases() -> Result<Vec<Case>, Box<dyn Error>> {
    let shell = primitives::cube(Vec3::new(200.0, 100.0, 150.0), [12, 12, 12]);
    let cutters = joined(&[
        moved(primitives::cylinder(20.0, 300.0, 32, 1), Vec3::new(-40.0, 0.0, 10.0)),
        moved(primitives::cylinder(25.0, 300.0, 32, 1), Vec3::new(45.0, 0.0, -15.0)),
    ])?;
    let big = primitives::sphere(100.0, 224);
    let small = moved(primitives::sphere(80.0, 224), Vec3::new(60.0, 20.0, 0.0));
    let scan = primitives::sphere(100.0, 1000);
    let drill = moved(primitives::cylinder(30.0, 300.0, 48, 1), Vec3::new(20.0, 0.0, 10.0));
    Ok(vec![
        ("product shell minus two cylinders", shell, cutters, 21),
        ("two spheres of 50 000 triangles", big, small, 5),
        ("a million triangles, drilled", scan, drill, 3),
    ])
}

fn main() -> Result<(), Box<dyn Error>> {
    println!("Modelling budget (medians)");
    for segments in [48, 96] {
        let (faces, time) = live_generator(segments)?;
        println!(
            "  live Subdivision Surface, 2 levels, cage of {faces:>6} faces: {} a frame",
            ms(time)
        );
    }
    let (triangles, time) = boolean_floor();
    println!(
        "  two spheres of {triangles:>7} triangles in all: triangulated, BVH, topology: {}",
        ms(time)
    );
    let (points, time) = symmetry_by_hand()?;
    println!(
        "  half a mesh mirrored, joined and welded ({points:>7} points): {}",
        ms(time)
    );
    let (triangles, time) = reduce_floor();
    println!("  a scan of {triangles:>8} triangles: topology and BVH: {}", ms(time));
    println!("Boolean (Subtract), the spike's two methods");
    for (name, a, b, runs) in boolean_cases()? {
        let triangles = |m: &PolyMesh| m.faces().map(|f| f.len() - 2).sum::<usize>();
        println!("  {name} ({} + {} triangles):", triangles(&a), triangles(&b));
        for method in [Method::Exact, Method::Rounded] {
            let mut last = None;
            let time = median(runs, || last = Some(boolean(&a, &b, BooleanMode::Subtract, method)));
            let Some(Ok(result)) = last else {
                println!("    {method:?}: failed");
                continue;
            };
            println!(
                "    {method:<7?} {}  closed {:<5}  {} faces, {} pieces, {} pairs, {} exact signs",
                ms(time),
                is_closed(&result.mesh),
                result.mesh.face_count(),
                result.stats.pieces,
                result.stats.pairs,
                result.stats.exact_calls
            );
            let t = result.stats.times.map(|d| d.as_secs_f64() * 1000.0);
            println!(
                "            stages (ms): prepare {:.1}, cut {:.1}, join {:.1}, stitch {:.1}, classify {:.1}, build {:.1}",
                t[0], t[1], t[2], t[3], t[4], t[5]
            );
        }
    }
    Ok(())
}
