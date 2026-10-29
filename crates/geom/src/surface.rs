//! Surfaces made from splines: the Extrude generator's solid (see
//! `docs/reference/generator-extrude.md`). Pure functions of a sampled
//! [`Polyline`]; Lathe, Sweep and Loft will join them.

use glam::{Vec2, Vec3};

use crate::{PolyMesh, spline::Polyline, triangulate_outlines, u32_index};

/// How an extrusion finds its movement.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ExtrudeDirection {
    /// Along the axis closest to the perpendicular of the spline's plane.
    #[default]
    Auto,
    X,
    Y,
    Z,
    /// Along [`Extrusion::movement`]'s direction.
    Custom,
    /// Exactly [`Extrusion::movement`].
    Absolute,
}

impl ExtrudeDirection {
    pub const ALL: [Self; 6] = [Self::Auto, Self::X, Self::Y, Self::Z, Self::Custom, Self::Absolute];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::X => "X",
            Self::Y => "Y",
            Self::Z => "Z",
            Self::Custom => "Custom",
            Self::Absolute => "Absolute",
        }
    }
}

/// How caps are cut into polygons.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CapType {
    /// One polygon per outline without holes; triangles where holes need them.
    #[default]
    NGons,
    /// Every cap triangulated.
    Triangles,
}

impl CapType {
    pub const ALL: [Self; 2] = [Self::NGons, Self::Triangles];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::NGons => "N-Gons",
            Self::Triangles => "Triangles",
        }
    }
}

/// The Extrude generator's options: how far a spline is pushed, and how its
/// solid is closed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Extrusion {
    pub direction: ExtrudeDirection,
    /// Length of the movement (cm) for every direction but Absolute; negative
    /// reverses it.
    pub offset: f32,
    /// Custom: the direction; Absolute: the movement itself (cm).
    pub movement: Vec3,
    /// Rings of polygons along the movement (1 – [`Extrusion::MAX_SUBDIVISION`]).
    pub subdivision: u32,
    /// Turns the shell inside out (caps untouched).
    pub reverse_normals: bool,
    pub start_cap: bool,
    pub end_cap: bool,
    pub cap_type: CapType,
}

impl Default for Extrusion {
    fn default() -> Self {
        Self {
            direction: ExtrudeDirection::Auto,
            offset: 20.0,
            movement: Vec3::new(0.0, 0.0, 20.0),
            subdivision: 1,
            reverse_normals: false,
            start_cap: true,
            end_cap: true,
            cap_type: CapType::NGons,
        }
    }
}

impl Extrusion {
    pub const MAX_SUBDIVISION: u32 = 4000;

    /// The movement for `polyline` (in the polyline's space).
    #[must_use]
    pub fn vector(&self, polyline: &Polyline) -> Vec3 {
        match self.direction {
            ExtrudeDirection::Auto => plane_axis(polyline) * self.offset,
            ExtrudeDirection::X => Vec3::X * self.offset,
            ExtrudeDirection::Y => Vec3::Y * self.offset,
            ExtrudeDirection::Z => Vec3::Z * self.offset,
            ExtrudeDirection::Custom => self.movement.normalize_or_zero() * self.offset,
            ExtrudeDirection::Absolute => self.movement,
        }
    }
}

/// The axis (X, Y or Z, positive) closest to the perpendicular of the plane the
/// curve lies in; Z for a straight line.
fn plane_axis(polyline: &Polyline) -> Vec3 {
    let normal: Vec3 = polyline
        .runs()
        .map(|(points, _)| crate::mesh::newell(points.iter().copied()))
        .sum();
    let a = normal.abs();
    if a.max_element() <= f32::EPSILON || (a.z >= a.x && a.z >= a.y) {
        Vec3::Z
    } else if a.x >= a.y {
        Vec3::X
    } else {
        Vec3::Y
    }
}

/// One run of the polyline as the solid uses it.
struct Run<'a> {
    points: &'a [Vec3],
    closed: bool,
    /// Projected onto the plane across the movement.
    flat: Vec<Vec2>,
    /// Closed runs: how many other closed runs contain it (even: an outline,
    /// odd: a hole).
    depth: usize,
    /// Its points in the order that faces outwards.
    order: Vec<u32>,
    /// Index of its first point in the mesh.
    base: u32,
}

/// The solid `polyline` makes when it is pushed along `extrusion`'s movement:
/// a shell of `subdivision` rings per run, and caps on its closed outlines with
/// their holes cut out. Empty for a movement of zero length or a polyline
/// without runs of two points.
#[must_use]
pub fn extrude(polyline: &Polyline, extrusion: &Extrusion) -> PolyMesh {
    let movement = extrusion.vector(polyline);
    let Some(dir) = movement.try_normalize().filter(|_| movement.is_finite()) else {
        return PolyMesh::new();
    };
    // u × v = dir: counter-clockwise in 2D is counter-clockwise around dir.
    let u = dir.any_orthonormal_vector();
    let v = dir.cross(u);
    let rings = extrusion.subdivision.clamp(1, Extrusion::MAX_SUBDIVISION);
    let mut runs: Vec<Run<'_>> = polyline
        .runs()
        .filter(|(points, _)| points.len() >= 2)
        .map(|(points, closed)| Run {
            points,
            closed: closed && points.len() >= 3,
            flat: points.iter().map(|&p| Vec2::new(p.dot(u), p.dot(v))).collect(),
            depth: 0,
            order: (0..u32_index(points.len())).collect(),
            base: 0,
        })
        .collect();
    // Nesting (even-odd) and orientation: outlines counter-clockwise, holes clockwise.
    for i in 0..runs.len() {
        if !runs[i].closed {
            continue;
        }
        let probe = runs[i].flat[0];
        runs[i].depth = (0..runs.len())
            .filter(|&j| j != i && runs[j].closed && contains(&runs[j].flat, probe))
            .count();
        let counter_clockwise = area(&runs[i].flat) > 0.0;
        if counter_clockwise != runs[i].depth.is_multiple_of(2) {
            runs[i].order.reverse();
        }
    }

    let mut points = Vec::new();
    let mut faces: Vec<Vec<u32>> = Vec::new();
    let steps = f32::from(u16::try_from(rings).unwrap_or(u16::MAX));
    for run in &mut runs {
        run.base = u32_index(points.len());
        for k in 0..=rings {
            let along = movement * (f32::from(u16::try_from(k).unwrap_or(u16::MAX)) / steps);
            points.extend(run.points.iter().map(|&p| p + along));
        }
    }
    let n_of = |run: &Run<'_>| u32_index(run.points.len());
    let at = |run: &Run<'_>, ring: u32, local: u32| run.base + ring * n_of(run) + local;
    // The shell: each edge of each run swept ring by ring.
    for run in &runs {
        let n = run.order.len();
        let edges = if run.closed { n } else { n - 1 };
        for e in 0..edges {
            let (a, b) = (run.order[e], run.order[(e + 1) % n]);
            for k in 0..rings {
                let mut quad = vec![at(run, k, a), at(run, k, b), at(run, k + 1, b), at(run, k + 1, a)];
                if extrusion.reverse_normals {
                    quad.reverse();
                }
                faces.push(quad);
            }
        }
    }
    // The caps: each outline with its direct holes.
    for (i, outline) in runs.iter().enumerate() {
        if !outline.closed || !outline.depth.is_multiple_of(2) {
            continue;
        }
        let holes: Vec<&Run<'_>> = runs
            .iter()
            .enumerate()
            .filter(|&(j, hole)| {
                j != i && hole.closed && hole.depth == outline.depth + 1 && contains(&outline.flat, hole.flat[0])
            })
            .map(|(_, hole)| hole)
            .collect();
        // Cap polygons around dir, as (run, local point) pairs.
        let mut cap: Vec<Vec<(&Run<'_>, u32)>> = Vec::new();
        if holes.is_empty() && extrusion.cap_type == CapType::NGons {
            cap.push(outline.order.iter().map(|&p| (outline, p)).collect());
        } else {
            let hole_flats: Vec<&[Vec2]> = holes.iter().map(|h| h.flat.as_slice()).collect();
            let mut owners: Vec<(&Run<'_>, u32)> = (0..n_of(outline)).map(|p| (outline, p)).collect();
            for hole in &holes {
                owners.extend((0..n_of(hole)).map(|p| (*hole, p)));
            }
            for triangle in triangulate_outlines(&outline.flat, &hole_flats) {
                cap.push(
                    triangle
                        .iter()
                        .filter_map(|&c| owners.get(crate::us(c)).copied())
                        .collect(),
                );
            }
        }
        for polygon in &cap {
            if extrusion.end_cap {
                faces.push(polygon.iter().map(|&(run, p)| at(run, rings, p)).collect());
            }
            if extrusion.start_cap {
                faces.push(polygon.iter().rev().map(|&(run, p)| at(run, 0, p)).collect());
            }
        }
    }
    let mut mesh = PolyMesh::from_faces(points, Vec::<Vec<u32>>::new()).unwrap_or_default();
    for face in &faces {
        // Corners that coincide (a triangle across a bridge) make no face.
        let _ = mesh.add_face(face);
    }
    mesh
}

/// Twice the signed area: positive when counter-clockwise.
fn area(polygon: &[Vec2]) -> f32 {
    let n = polygon.len();
    (0..n).map(|i| polygon[i].perp_dot(polygon[(i + 1) % n])).sum()
}

/// Even-odd point-in-polygon test.
fn contains(polygon: &[Vec2], p: Vec2) -> bool {
    let n = polygon.len();
    let mut inside = false;
    for i in 0..n {
        let (a, b) = (polygon[i], polygon[(i + n - 1) % n]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spline::Segment;

    fn polyline(runs: &[(&[Vec3], bool)]) -> Polyline {
        Polyline {
            points: runs.iter().flat_map(|(p, _)| p.iter().copied()).collect(),
            segments: runs
                .iter()
                .map(|&(p, closed)| Segment {
                    count: u32_index(p.len()),
                    closed,
                })
                .collect(),
        }
    }

    /// A square of side `size` around the origin in the XY plane, counter-clockwise
    /// seen from +Z (or clockwise).
    fn square(size: f32, counter_clockwise: bool) -> Vec<Vec3> {
        let h = size * 0.5;
        let mut points = vec![
            Vec3::new(-h, -h, 0.0),
            Vec3::new(h, -h, 0.0),
            Vec3::new(h, h, 0.0),
            Vec3::new(-h, h, 0.0),
        ];
        if !counter_clockwise {
            points.reverse();
        }
        points
    }

    fn circle(radius: f32, n: u32) -> Vec<Vec3> {
        (0..n)
            .map(|i| {
                let a =
                    std::f32::consts::TAU * f32::from(u16::try_from(i).unwrap()) / f32::from(u16::try_from(n).unwrap());
                Vec3::new(radius * a.cos(), radius * a.sin(), 0.0)
            })
            .collect()
    }

    /// The enclosed volume (divergence theorem over the triangulated faces):
    /// positive when every face points outwards.
    fn volume(mesh: &PolyMesh) -> f32 {
        let mut triangles = Vec::new();
        let mut triangulator = crate::Triangulator::new();
        for f in 0..mesh.face_count() {
            triangulator.face_triangles(mesh, f, &mut triangles);
        }
        triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| mesh.points()[crate::us(i)]);
                a.dot(b.cross(c)) / 6.0
            })
            .sum()
    }

    /// Every edge used by exactly two faces, once each way.
    fn is_closed(mesh: &PolyMesh) -> bool {
        let mut edges = std::collections::HashMap::new();
        for face in mesh.faces() {
            for i in 0..face.len() {
                *edges.entry((face[i], face[(i + 1) % face.len()])).or_insert(0) += 1;
            }
        }
        edges
            .iter()
            .all(|(&(a, b), &count)| count == 1 && edges.get(&(b, a)) == Some(&1))
    }

    #[test]
    fn a_circle_becomes_a_closed_cylinder_facing_outwards() {
        for counter_clockwise in [true, false] {
            let mut points = circle(100.0, 32);
            if !counter_clockwise {
                points.reverse();
            }
            let extrusion = Extrusion {
                subdivision: 3,
                ..Extrusion::default()
            };
            let mesh = extrude(&polyline(&[(&points, true)]), &extrusion);
            assert_eq!(mesh.point_count(), 32 * 4, "four rings");
            assert_eq!(mesh.face_count(), 32 * 3 + 2, "shell quads and two n-gon caps");
            assert!(is_closed(&mesh));
            let expected = 32.0 * 0.5 * 100.0 * 100.0 * (std::f32::consts::TAU / 32.0).sin() * 20.0;
            assert!(
                (volume(&mesh) - expected).abs() < 1.0,
                "{} vs {expected}",
                volume(&mesh)
            );
            let (min, max) = mesh.bounds().unwrap();
            assert_eq!((min.z, max.z), (0.0, 20.0), "along +Z for a spline in XY");
        }
    }

    #[test]
    fn holes_are_cut_out_of_the_caps_whichever_way_they_run() {
        for (outer, inner) in [(true, true), (true, false), (false, true), (false, false)] {
            let (a, b) = (square(200.0, outer), square(100.0, inner));
            let mesh = extrude(&polyline(&[(&a, true), (&b, true)]), &Extrusion::default());
            assert!(is_closed(&mesh), "{outer} {inner}");
            let expected = (200.0 * 200.0 - 100.0 * 100.0) * 20.0;
            assert!(
                (volume(&mesh) - expected).abs() < 1.0,
                "{outer} {inner}: {}",
                volume(&mesh)
            );
            // The cap is triangles: 8 corners → 8 triangles per cap.
            let triangles = mesh.faces().filter(|f| f.len() == 3).count();
            assert_eq!(triangles, 16);
        }
    }

    #[test]
    fn islands_inside_holes_get_their_own_caps() {
        let (a, b, c) = (square(300.0, true), square(200.0, true), square(100.0, true));
        let mesh = extrude(&polyline(&[(&a, true), (&b, true), (&c, true)]), &Extrusion::default());
        assert!(is_closed(&mesh));
        let expected = (300.0f32 * 300.0 - 200.0 * 200.0 + 100.0 * 100.0) * 20.0;
        assert!((volume(&mesh) - expected).abs() < 1.0, "{}", volume(&mesh));
    }

    #[test]
    fn directions() {
        let xy = polyline(&[(&square(100.0, true), true)]);
        let zy = Polyline {
            points: xy.points.iter().map(|p| Vec3::new(0.0, p.y, p.x)).collect(),
            ..xy.clone()
        };
        let xz = Polyline {
            points: xy.points.iter().map(|p| Vec3::new(p.x, 0.0, p.y)).collect(),
            ..xy.clone()
        };
        let auto = Extrusion::default();
        assert_eq!(auto.vector(&xy), Vec3::new(0.0, 0.0, 20.0));
        assert_eq!(auto.vector(&zy), Vec3::new(20.0, 0.0, 0.0));
        assert_eq!(auto.vector(&xz), Vec3::new(0.0, 20.0, 0.0));
        let with = |direction, offset| Extrusion {
            direction,
            offset,
            movement: Vec3::new(3.0, 4.0, 0.0),
            ..Extrusion::default()
        };
        assert_eq!(with(ExtrudeDirection::Y, -5.0).vector(&xy), Vec3::new(0.0, -5.0, 0.0));
        assert!(
            with(ExtrudeDirection::Custom, 10.0)
                .vector(&xy)
                .abs_diff_eq(Vec3::new(6.0, 8.0, 0.0), 1e-5)
        );
        assert_eq!(
            with(ExtrudeDirection::Absolute, 99.0).vector(&xy),
            Vec3::new(3.0, 4.0, 0.0)
        );
        // A negative offset goes the other way and still faces outwards.
        let back = extrude(&xy, &with(ExtrudeDirection::Auto, -20.0));
        assert!(volume(&back) > 0.0);
        assert_eq!(back.bounds().unwrap().0.z, -20.0);
        // Zero movement: nothing.
        assert_eq!(extrude(&xy, &with(ExtrudeDirection::Auto, 0.0)).face_count(), 0);
    }

    #[test]
    fn open_splines_get_a_shell_that_reverse_normals_flips() {
        let line = [Vec3::ZERO, Vec3::new(100.0, 0.0, 0.0), Vec3::new(100.0, 100.0, 0.0)];
        let open = polyline(&[(&line, false)]);
        let mesh = extrude(&open, &Extrusion::default());
        assert_eq!(mesh.face_count(), 2, "two quads, no caps");
        let flipped = extrude(
            &open,
            &Extrusion {
                reverse_normals: true,
                ..Extrusion::default()
            },
        );
        assert!(mesh.face_normal(0).dot(flipped.face_normal(0)) < -0.99);
    }

    #[test]
    fn cap_options() {
        let square = polyline(&[(&square(100.0, true), true)]);
        let triangles = extrude(
            &square,
            &Extrusion {
                cap_type: CapType::Triangles,
                ..Extrusion::default()
            },
        );
        assert_eq!(triangles.faces().filter(|f| f.len() == 3).count(), 4);
        assert!(is_closed(&triangles));
        let open = extrude(
            &square,
            &Extrusion {
                start_cap: false,
                end_cap: false,
                ..Extrusion::default()
            },
        );
        assert_eq!(open.face_count(), 4, "the shell alone");
        let end_only = extrude(
            &square,
            &Extrusion {
                start_cap: false,
                ..Extrusion::default()
            },
        );
        assert_eq!(end_only.face_count(), 5);
        assert!(
            end_only.face_normal(4).abs_diff_eq(Vec3::Z, 1e-5),
            "the end cap faces forward"
        );
    }

    #[test]
    fn outlines_with_holes_triangulate_without_overlap() {
        let outer = [
            Vec2::new(0.0, 0.0),
            Vec2::new(10.0, 0.0),
            Vec2::new(10.0, 10.0),
            Vec2::new(0.0, 10.0),
        ];
        let hole_a: Vec<Vec2> = [(2.0, 2.0), (4.0, 2.0), (4.0, 4.0), (2.0, 4.0)]
            .map(|(x, y)| Vec2::new(x, y))
            .to_vec();
        let hole_b: Vec<Vec2> = [(6.0, 6.0), (8.0, 6.0), (8.0, 8.0), (6.0, 8.0)]
            .map(|(x, y)| Vec2::new(x, y))
            .to_vec();
        let triangles = triangulate_outlines(&outer, &[&hole_a, &hole_b]);
        let all: Vec<Vec2> = outer.iter().chain(&hole_a).chain(&hole_b).copied().collect();
        let mut total = 0.0;
        for t in &triangles {
            let [a, b, c] = t.map(|i| all[crate::us(i)]);
            let doubled = (b - a).perp_dot(c - a);
            assert!(doubled >= 0.0, "counter-clockwise");
            total += doubled * 0.5;
        }
        assert!((total - (100.0 - 4.0 - 4.0)).abs() < 1e-3, "{total}");
    }
}
