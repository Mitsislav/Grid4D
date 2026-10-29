//! Cloner layouts (see `docs/reference/mograph-cloner.md`): where each clone
//! goes, in the cloner's space, and which child it repeats.

use std::f32::consts::TAU;

use glam::{EulerRot, Mat3, Mat4, Quat, Vec3};

use crate::{PolyMesh, spline::SplinePlane};

/// How Linear and Grid Array values are read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum StepMode {
    /// From one clone to the next.
    #[default]
    PerStep,
    /// From the first clone to the last (Grid Array: the whole grid's size).
    EndPoint,
}

impl StepMode {
    pub const ALL: [Self; 2] = [Self::PerStep, Self::EndPoint];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::PerStep => "Per Step",
            Self::EndPoint => "End Point",
        }
    }
}

/// Clones in a row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Linear {
    pub count: u32,
    /// Steps skipped at the start (the row begins further along).
    pub offset: u32,
    pub mode: StepMode,
    /// Position change (cm).
    pub position: Vec3,
    /// Rotation change, HPB (radians).
    pub rotation: Vec3,
    /// Scale change (1.0 = 100 %).
    pub scale: Vec3,
}

impl Default for Linear {
    fn default() -> Self {
        Self {
            count: 3,
            offset: 0,
            mode: StepMode::PerStep,
            position: Vec3::new(0.0, 250.0, 0.0),
            rotation: Vec3::ZERO,
            scale: Vec3::ONE,
        }
    }
}

/// Clones on a circle around the cloner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Radial {
    pub count: u32,
    pub radius: f32,
    pub plane: SplinePlane,
    /// Each clone turned about the circle's axis by its angle.
    pub align: bool,
    /// Radians.
    pub start_angle: f32,
    /// Radians.
    pub end_angle: f32,
    /// Every clone moved along the circle (radians).
    pub offset: f32,
}

impl Default for Radial {
    fn default() -> Self {
        Self {
            count: 6,
            radius: 400.0,
            plane: SplinePlane::XZ,
            align: true,
            start_angle: 0.0,
            end_angle: TAU,
            offset: 0.0,
        }
    }
}

/// Clones in a box-shaped grid, centred on the cloner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridArray {
    /// Clones along X, Y and Z.
    pub count: [u32; 3],
    pub mode: StepMode,
    /// Per Step: the spacing; End Point: the whole grid's size (cm).
    pub size: Vec3,
}

impl Default for GridArray {
    fn default() -> Self {
        Self {
            count: [3, 3, 3],
            mode: StepMode::PerStep,
            size: Vec3::splat(250.0),
        }
    }
}

/// Where on an object Object mode puts the clones.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Distribution {
    /// On every point.
    #[default]
    Vertex,
    /// On every polygon's centre.
    PolygonCenter,
}

impl Distribution {
    pub const ALL: [Self; 2] = [Self::Vertex, Self::PolygonCenter];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Vertex => "Vertex",
            Self::PolygonCenter => "Polygon Center",
        }
    }
}

/// Clones on another object's surface (which object is the scene's business).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Surface {
    pub distribution: Distribution,
    /// Each clone's Y axis turned along the surface's normal there.
    pub align: bool,
}

impl Default for Surface {
    fn default() -> Self {
        Self {
            distribution: Distribution::Vertex,
            align: true,
        }
    }
}

/// The layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Layout {
    Linear(Linear),
    Radial(Radial),
    Grid(GridArray),
    Object(Surface),
}

impl Layout {
    /// The labels the Mode menu shows, in order.
    pub const LABELS: [&'static str; 4] = ["Linear", "Radial", "Grid Array", "Object"];

    #[must_use]
    pub const fn label(&self) -> &'static str {
        Self::LABELS[self.index()]
    }

    #[must_use]
    pub const fn index(&self) -> usize {
        match self {
            Self::Linear(_) => 0,
            Self::Radial(_) => 1,
            Self::Grid(_) => 2,
            Self::Object(_) => 3,
        }
    }

    /// The default layout of mode `index` (see [`Layout::LABELS`]).
    #[must_use]
    pub fn of_index(index: usize) -> Self {
        match index {
            1 => Self::Radial(Radial::default()),
            2 => Self::Grid(GridArray::default()),
            3 => Self::Object(Surface::default()),
            _ => Self::Linear(Linear::default()),
        }
    }
}

/// Which child each clone repeats.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CloneOrder {
    /// The children in turn.
    #[default]
    Iterate,
    /// A child picked at random (by the seed) for each clone.
    Random,
}

impl CloneOrder {
    pub const ALL: [Self; 2] = [Self::Iterate, Self::Random];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Iterate => "Iterate",
            Self::Random => "Random",
        }
    }
}

/// A Cloner's settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cloner {
    pub layout: Layout,
    pub clones: CloneOrder,
    pub seed: u32,
    /// The children's own placement is ignored (each clone sits exactly where
    /// the layout puts it); off, it is added on top.
    pub reset_coordinates: bool,
}

impl Default for Cloner {
    fn default() -> Self {
        Self {
            layout: Layout::Linear(Linear::default()),
            clones: CloneOrder::Iterate,
            seed: 1_234_567,
            reset_coordinates: true,
        }
    }
}

impl Cloner {
    /// Most clones per axis and per row or circle; the grid at most this many
    /// in all.
    pub const MAX_COUNT: u32 = 10_000;

    /// How many clones the layout makes (Object mode: as many as its object
    /// has points or polygons, so 0 here — see [`Cloner::placements`]).
    #[must_use]
    pub fn count(&self) -> usize {
        let count = match self.layout {
            Layout::Linear(linear) => linear.count,
            Layout::Radial(radial) => radial.count,
            Layout::Grid(grid) => grid
                .count
                .iter()
                .fold(1u32, |n, &c| n.saturating_mul(c))
                .min(Self::MAX_COUNT),
            Layout::Object(_) => 0,
        };
        usize::try_from(count.min(Self::MAX_COUNT)).unwrap_or(0)
    }

    /// Every clone's placement in the cloner's space, into `out`. Object mode
    /// needs `surface`: the object's mesh and the matrix taking it into the
    /// cloner's space (without one there are no clones); `normals` is scratch
    /// space. At most [`Cloner::MAX_COUNT`].
    pub fn placements(&self, surface: Option<(&PolyMesh, Mat4)>, normals: &mut Vec<Vec3>, out: &mut Vec<Mat4>) {
        out.clear();
        match self.layout {
            Layout::Object(layout) => {
                if let Some((mesh, to_cloner)) = surface {
                    surface_placements(mesh, to_cloner, layout, normals, out);
                }
            }
            Layout::Linear(_) | Layout::Radial(_) | Layout::Grid(_) => {
                out.extend((0..self.count()).map(|i| self.matrix(i)));
            }
        }
    }

    /// Clone `index`'s placement in the cloner's space (Linear, Radial and Grid
    /// Array; Object mode's come from [`Cloner::placements`]).
    #[must_use]
    pub fn matrix(&self, index: usize) -> Mat4 {
        let i = index as f32;
        let n = self.count();
        match self.layout {
            Layout::Linear(linear) => {
                // How far along: steps, or the fraction of the way to the end.
                let k = match linear.mode {
                    StepMode::PerStep => i + linear.offset as f32,
                    StepMode::EndPoint => (i + linear.offset as f32) / (n.max(2) - 1) as f32,
                };
                let rotation = linear.rotation * k;
                let scale = match linear.mode {
                    StepMode::PerStep => linear.scale.powf(k),
                    StepMode::EndPoint => Vec3::ONE.lerp(linear.scale, k),
                };
                Mat4::from_scale_rotation_translation(
                    scale,
                    Quat::from_euler(EulerRot::YXZ, rotation.x, rotation.y, rotation.z),
                    linear.position * k,
                )
            }
            Layout::Radial(radial) => {
                let span = radial.end_angle - radial.start_angle;
                // A full circle spaces the clones evenly round it; an arc puts
                // the last one at its end.
                let full = (span.abs() - TAU).abs() < 1e-4 || span.abs() > TAU;
                let steps = if full { n.max(1) } else { n.max(2) - 1 };
                let angle = radial.start_angle + radial.offset + span * i / steps as f32;
                let axis = radial.plane.place(Vec3::Z);
                let position = radial.plane.place(Vec3::new(angle.cos(), angle.sin(), 0.0)) * radial.radius;
                let rotation = if radial.align {
                    Quat::from_axis_angle(axis, angle)
                } else {
                    Quat::IDENTITY
                };
                Mat4::from_rotation_translation(rotation, position)
            }
            Layout::Grid(grid) => {
                let [nx, ny, nz] = grid.count.map(|c| c.max(1) as usize);
                let at = [index % nx, (index / nx) % ny, index / (nx * ny) % nz];
                let mut position = Vec3::ZERO;
                for axis in 0..3 {
                    let count = [nx, ny, nz][axis];
                    let step = match grid.mode {
                        StepMode::PerStep => grid.size[axis],
                        StepMode::EndPoint if count > 1 => grid.size[axis] / (count - 1) as f32,
                        StepMode::EndPoint => 0.0,
                    };
                    position[axis] = (at[axis] as f32 - (count - 1) as f32 * 0.5) * step;
                }
                Mat4::from_translation(position)
            }
            Layout::Object(_) => Mat4::IDENTITY,
        }
    }

    /// Which of `children` children clone `index` repeats.
    #[must_use]
    pub fn source(&self, index: usize, children: usize) -> usize {
        if children == 0 {
            return 0;
        }
        match self.clones {
            CloneOrder::Iterate => index % children,
            CloneOrder::Random => {
                let hash = mix(u64::from(self.seed) ^ (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
                usize::try_from(hash % children as u64).unwrap_or(0)
            }
        }
    }
}

/// Object mode: a placement on every point (its normal: the area-weighted
/// mean of its polygons') or polygon centre of `mesh`, taken into the cloner's
/// space by `to_cloner`.
fn surface_placements(mesh: &PolyMesh, to_cloner: Mat4, layout: Surface, normals: &mut Vec<Vec3>, out: &mut Vec<Mat4>) {
    let linear = Mat3::from_mat4(to_cloner);
    let normal_matrix = if linear.determinant().abs() > f32::EPSILON {
        linear.inverse().transpose()
    } else {
        Mat3::IDENTITY
    };
    let place = |point: Vec3, normal: Vec3| {
        let normal = (normal_matrix * normal).normalize_or_zero();
        let rotation = if layout.align && normal != Vec3::ZERO {
            Quat::from_rotation_arc(Vec3::Y, normal)
        } else {
            Quat::IDENTITY
        };
        Mat4::from_rotation_translation(rotation, to_cloner.transform_point3(point))
    };
    let max = usize::try_from(Cloner::MAX_COUNT).unwrap_or(usize::MAX);
    match layout.distribution {
        Distribution::Vertex => {
            normals.clear();
            normals.resize(mesh.point_count(), Vec3::ZERO);
            for face in 0..mesh.face_count() {
                let vector = mesh.face_vector(face);
                for &corner in mesh.face(face) {
                    if let Some(normal) = usize::try_from(corner).ok().and_then(|c| normals.get_mut(c)) {
                        *normal += vector;
                    }
                }
            }
            out.extend(
                mesh.points()
                    .iter()
                    .zip(normals.iter())
                    .take(max)
                    .map(|(&point, &normal)| place(point, normal)),
            );
        }
        Distribution::PolygonCenter => {
            out.extend(
                (0..mesh.face_count())
                    .take(max)
                    .map(|face| place(mesh.face_center(face), mesh.face_normal(face))),
            );
        }
    }
}

/// A well-mixed 64-bit hash (SplitMix64's finaliser).
const fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn position(cloner: &Cloner, index: usize) -> Vec3 {
        cloner.matrix(index).w_axis.truncate()
    }

    #[test]
    fn a_linear_row_steps_or_spreads_to_its_end() {
        let mut linear = Linear {
            count: 4,
            position: Vec3::new(100.0, 0.0, 0.0),
            scale: Vec3::splat(0.5),
            ..Linear::default()
        };
        let cloner = |linear| Cloner {
            layout: Layout::Linear(linear),
            ..Cloner::default()
        };
        let row = cloner(linear);
        assert_eq!(row.count(), 4);
        assert!(position(&row, 3).abs_diff_eq(Vec3::new(300.0, 0.0, 0.0), 1e-4));
        let (scale, _, _) = row.matrix(2).to_scale_rotation_translation();
        assert!(scale.abs_diff_eq(Vec3::splat(0.25), 1e-5), "halved per step: {scale}");
        linear.mode = StepMode::EndPoint;
        let spread = cloner(linear);
        assert!(
            position(&spread, 3).abs_diff_eq(Vec3::new(100.0, 0.0, 0.0), 1e-4),
            "the last at the end"
        );
        assert!(position(&spread, 0).abs_diff_eq(Vec3::ZERO, 1e-4));
        linear.offset = 1;
        linear.mode = StepMode::PerStep;
        assert!(position(&cloner(linear), 0).abs_diff_eq(Vec3::new(100.0, 0.0, 0.0), 1e-4));
    }

    #[test]
    fn a_radial_ring_is_even_and_an_arc_ends_at_its_end() {
        let ring = Cloner {
            layout: Layout::Radial(Radial {
                count: 4,
                radius: 100.0,
                ..Radial::default()
            }),
            ..Cloner::default()
        };
        // On the floor (XZ): every clone 100 cm out, at Y = 0, a quarter turn apart.
        for i in 0..4 {
            let p = position(&ring, i);
            assert!((p.length() - 100.0).abs() < 1e-3 && p.y.abs() < 1e-4, "{p}");
        }
        assert!(position(&ring, 0).abs_diff_eq(Vec3::new(100.0, 0.0, 0.0), 1e-3));
        assert!(position(&ring, 2).abs_diff_eq(Vec3::new(-100.0, 0.0, 0.0), 1e-3));
        // Aligned: the clone's X axis turns with it.
        let x = ring.matrix(2).transform_vector3(Vec3::X);
        assert!(x.abs_diff_eq(Vec3::NEG_X, 1e-4), "{x}");
        let arc = Cloner {
            layout: Layout::Radial(Radial {
                count: 3,
                radius: 100.0,
                plane: SplinePlane::XY,
                end_angle: std::f32::consts::PI,
                ..Radial::default()
            }),
            ..Cloner::default()
        };
        assert!(position(&arc, 1).abs_diff_eq(Vec3::new(0.0, 100.0, 0.0), 1e-3));
        assert!(position(&arc, 2).abs_diff_eq(Vec3::new(-100.0, 0.0, 0.0), 1e-3));
    }

    #[test]
    fn a_grid_is_centred_and_end_point_fits_its_size() {
        let mut grid = GridArray {
            count: [3, 2, 1],
            size: Vec3::splat(100.0),
            ..GridArray::default()
        };
        let cloner = |grid| Cloner {
            layout: Layout::Grid(grid),
            ..Cloner::default()
        };
        let per_step = cloner(grid);
        assert_eq!(per_step.count(), 6);
        assert!(position(&per_step, 0).abs_diff_eq(Vec3::new(-100.0, -50.0, 0.0), 1e-4));
        assert!(position(&per_step, 5).abs_diff_eq(Vec3::new(100.0, 50.0, 0.0), 1e-4));
        grid.mode = StepMode::EndPoint;
        let fitted = cloner(grid);
        assert!(position(&fitted, 5).abs_diff_eq(Vec3::new(50.0, 50.0, 0.0), 1e-4));
        grid.count = [Cloner::MAX_COUNT; 3];
        assert_eq!(cloner(grid).count(), Cloner::MAX_COUNT as usize, "capped");
    }

    #[test]
    fn object_mode_places_clones_on_points_or_polygon_centres_along_normals() {
        let cube = crate::shapes::cube(100.0);
        let cloner = |distribution, align| Cloner {
            layout: Layout::Object(Surface { distribution, align }),
            ..Cloner::default()
        };
        let (mut normals, mut out) = (Vec::new(), Vec::new());
        let lifted = Mat4::from_translation(Vec3::Y * 10.0);
        cloner(Distribution::PolygonCenter, true).placements(Some((&cube, lifted)), &mut normals, &mut out);
        assert_eq!(out.len(), 6);
        for m in &out {
            // Each clone's Y axis points out of its face, from the face's centre.
            let (centre, up) = (m.w_axis.truncate() - Vec3::Y * 10.0, m.transform_vector3(Vec3::Y));
            assert!((centre.length() - 50.0).abs() < 1e-3, "{centre}");
            assert!(up.abs_diff_eq(centre / 50.0, 1e-4), "{up} at {centre}");
        }
        cloner(Distribution::Vertex, true).placements(Some((&cube, Mat4::IDENTITY)), &mut normals, &mut out);
        assert_eq!(out.len(), 8);
        let corner = out[0].w_axis.truncate();
        let up = out[0].transform_vector3(Vec3::Y);
        assert!(
            up.abs_diff_eq(corner.normalize(), 1e-4),
            "a corner's normal points out of it"
        );
        cloner(Distribution::Vertex, false).placements(Some((&cube, Mat4::IDENTITY)), &mut normals, &mut out);
        assert_eq!(out[0].transform_vector3(Vec3::Y), Vec3::Y, "unaligned");
        cloner(Distribution::Vertex, true).placements(None, &mut normals, &mut out);
        assert!(out.is_empty(), "no object, no clones");
        Cloner::default().placements(None, &mut normals, &mut out);
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn clones_iterate_or_pick_children_by_seed() {
        let mut cloner = Cloner::default();
        let order: Vec<usize> = (0..5).map(|i| cloner.source(i, 2)).collect();
        assert_eq!(order, [0, 1, 0, 1, 0]);
        cloner.clones = CloneOrder::Random;
        let a: Vec<usize> = (0..64).map(|i| cloner.source(i, 3)).collect();
        assert!(a.iter().all(|&s| s < 3));
        assert!((0..3).all(|s| a.contains(&s)), "every child used");
        assert_eq!(
            a,
            (0..64).map(|i| cloner.source(i, 3)).collect::<Vec<_>>(),
            "repeatable"
        );
        cloner.seed += 1;
        assert_ne!(
            a,
            (0..64).map(|i| cloner.source(i, 3)).collect::<Vec<_>>(),
            "the seed varies it"
        );
        assert_eq!(cloner.source(3, 0), 0);
    }
}
