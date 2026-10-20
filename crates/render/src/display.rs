//! How a view draws objects: the shading and line style of its Display menu.

use grid4d_geom::{PolyMesh, Topology, primitives};

/// The shading modes. The "(Lines)" ones draw the line style's lines over the
/// shaded surfaces; Hidden Line and Lines always draw lines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShadingMode {
    /// Smooth and lit.
    #[default]
    Gouraud,
    GouraudLines,
    /// Lit by the default light only, without highlights.
    Quick,
    QuickLines,
    /// The object's colour, unlit.
    Constant,
    ConstantLines,
    /// Lines; the surfaces are not drawn but hide what is behind them.
    HiddenLine,
    /// Lines only, hidden ones included.
    Lines,
}

/// How the surfaces of a [`ShadingMode`] are filled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fill {
    Lit,
    Quick,
    Constant,
    /// Depth only: invisible, but hiding lines behind them.
    DepthOnly,
    None,
}

impl Fill {
    /// How the mesh shader numbers the lit fills.
    pub(crate) const fn code(self) -> f32 {
        match self {
            Self::Lit | Self::DepthOnly | Self::None => 0.0,
            Self::Quick => 1.0,
            Self::Constant => 2.0,
        }
    }
}

impl ShadingMode {
    pub const ALL: [Self; 8] = [
        Self::Gouraud,
        Self::GouraudLines,
        Self::Quick,
        Self::QuickLines,
        Self::Constant,
        Self::ConstantLines,
        Self::HiddenLine,
        Self::Lines,
    ];

    pub(crate) const fn fill(self) -> Fill {
        match self {
            Self::Gouraud | Self::GouraudLines => Fill::Lit,
            Self::Quick | Self::QuickLines => Fill::Quick,
            Self::Constant | Self::ConstantLines => Fill::Constant,
            Self::HiddenLine => Fill::DepthOnly,
            Self::Lines => Fill::None,
        }
    }

    /// Whether the mode draws lines.
    #[must_use]
    pub const fn lines(self) -> bool {
        !matches!(self, Self::Gouraud | Self::Quick | Self::Constant)
    }

    /// Lines in the object's colour (no surfaces to stand out against) rather
    /// than a dark shade of it.
    pub(crate) const fn coloured_lines(self) -> bool {
        matches!(self, Self::HiddenLine | Self::Lines)
    }
}

/// What is drawn of each object: the object itself, or its bounding box.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DisplayStyle {
    /// The object; lines along its polygons' edges.
    #[default]
    Wireframe,
    /// The object's bounding box, in its own orientation.
    Box,
}

/// A view's display mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DisplayMode {
    pub shading: ShadingMode,
    pub style: DisplayStyle,
}

/// `mesh`'s bounding box as a six-sided polygon mesh (`None` without points). Flat
/// sides get a hair of thickness, so every face has a normal.
#[must_use]
pub(crate) fn bounding_box(mesh: &PolyMesh) -> Option<PolyMesh> {
    let (min, max) = mesh.bounds()?;
    let size = max - min;
    let thickness = size.max_element().max(1.0) * 1e-4;
    let mut cuboid = primitives::cube(size.max(glam::Vec3::splat(thickness)), [1, 1, 1]);
    let centre = (min + max) * 0.5;
    for point in cuboid.points_mut() {
        *point += centre;
    }
    Some(cuboid)
}

/// The points and edges (pairs of point indices) of `mesh`'s polygons, for a
/// line list. Both vectors are cleared first.
pub(crate) fn edge_lines(mesh: &PolyMesh, points: &mut Vec<[f32; 3]>, indices: &mut Vec<u32>) {
    points.clear();
    points.extend(mesh.points().iter().map(|p| p.to_array()));
    indices.clear();
    indices.extend(Topology::build(mesh).edges().iter().flatten());
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::*;

    #[test]
    fn modes_fill_and_draw_lines_as_the_menu_says() {
        use ShadingMode as S;
        let table: Vec<(Fill, bool, bool)> = S::ALL
            .iter()
            .map(|m| (m.fill(), m.lines(), m.coloured_lines()))
            .collect();
        assert_eq!(
            table,
            [
                (Fill::Lit, false, false),
                (Fill::Lit, true, false),
                (Fill::Quick, false, false),
                (Fill::Quick, true, false),
                (Fill::Constant, false, false),
                (Fill::Constant, true, false),
                (Fill::DepthOnly, true, true),
                (Fill::None, true, true),
            ]
        );
        assert_eq!(DisplayMode::default().shading, S::Gouraud);
    }

    #[test]
    fn an_n_gon_shows_its_outline_not_its_triangles() {
        // A hexagon (one face) and a quad sharing an edge.
        let mut mesh = PolyMesh::new();
        for i in 0..6u8 {
            let a = f32::from(i) * std::f32::consts::TAU / 6.0;
            mesh.add_point(Vec3::new(a.cos(), 0.0, a.sin())).unwrap();
        }
        mesh.add_face(&[0, 1, 2, 3, 4, 5]).unwrap();
        let p = mesh.add_point(Vec3::new(2.0, 0.0, 0.0)).unwrap();
        let q = mesh.add_point(Vec3::new(2.0, 0.0, 1.0)).unwrap();
        mesh.add_face(&[1, 0, p, q]).unwrap();
        let (mut points, mut indices) = (Vec::new(), Vec::new());
        edge_lines(&mesh, &mut points, &mut indices);
        assert_eq!(points.len(), 8);
        assert_eq!(indices.len(), 2 * 9, "six around the hexagon, three more for the quad");
    }

    #[test]
    fn a_box_spans_the_bounds_and_a_flat_mesh_gets_a_thin_one() {
        let mesh = primitives::cylinder(1.0, 2.0, 12, 1);
        let cuboid = bounding_box(&mesh).unwrap();
        let (min, max) = cuboid.bounds().unwrap();
        let (bmin, bmax) = mesh.bounds().unwrap();
        assert!(
            min.abs_diff_eq(bmin, 1e-5) && max.abs_diff_eq(bmax, 1e-5),
            "{min} {max}"
        );
        assert_eq!((cuboid.point_count(), cuboid.face_count()), (8, 6));

        let plane = primitives::plane(4.0, 2.0, 3, 3);
        let thin = bounding_box(&plane).unwrap();
        let (min, max) = thin.bounds().unwrap();
        assert!(max.y - min.y > 0.0 && max.y - min.y < 0.01, "{min} {max}");
        assert!(thin.faces().enumerate().all(|(f, _)| thin.face_area(f) > 0.0));
        assert!(bounding_box(&PolyMesh::new()).is_none());
    }
}
