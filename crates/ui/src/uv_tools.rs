//! The UV tools on a polygon object (see `docs/reference/manager-uv-editor.md`):
//! each one changes the object's corner UVs (a new mesh handle, its components
//! kept) or its seams, on its selected polygons — all its visible ones when
//! none is selected. `commands::apply` records each as one undo step.

use grid4d_geom::{
    BitSet, selection,
    uv::{self, FlipAxis, PackOptions, Projection, RelaxOptions, UnwrapOptions},
};
use grid4d_scene::{Components, MeshHandle, NodeData, ObjectKind};

use crate::i18n::key;

/// A UV tool.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UvOp {
    /// Flattens the polygons along the seams (the selected edges count in
    /// Edges mode; with neither, edges sharper than the seam angle) and packs
    /// the parts.
    Unwrap,
    /// Evens out stretching.
    Relax,
    /// Lays the islands of the polygons into the square.
    Pack,
    /// Projects from a shape fitted to the object (Box, Spherical,
    /// Cylindrical, Flat).
    Project(Projection),
    /// Mirrors the polygons' UVs about their middle.
    Flip(FlipAxis),
    /// The selected edges become seams.
    MarkSeams,
    /// The selected edges stop being seams (all seams with no edge selected).
    ClearSeams,
}

impl UvOp {
    /// The undo step's name.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Unwrap => "Unwrap UVs",
            Self::Relax => "Relax UVs",
            Self::Pack => "Pack Islands",
            Self::Project(_) => "Project UVs",
            Self::Flip(FlipAxis::U) => "Flip U",
            Self::Flip(FlipAxis::V) => "Flip V",
            Self::MarkSeams => "Mark Seams",
            Self::ClearSeams => "Clear Seams",
        }
    }
}

/// The UV tools' options (editor state).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvSettings {
    /// Unwrap without seams cuts edges sharper than this, in degrees.
    pub seam_angle: f32,
    /// Packing leaves this many pixels between islands …
    pub margin_pixels: f32,
    /// … of a picture this many pixels wide.
    pub texture_size: u32,
    /// Packing may turn islands a quarter.
    pub rotate: bool,
    pub relax: RelaxOptions,
}

impl Default for UvSettings {
    fn default() -> Self {
        Self {
            seam_angle: 60.0,
            margin_pixels: 4.0,
            texture_size: 1024,
            rotate: true,
            relax: RelaxOptions::default(),
        }
    }
}

impl UvSettings {
    /// How islands are packed.
    #[must_use]
    pub fn pack(&self) -> PackOptions {
        let size = f32::from(u16::try_from(self.texture_size.clamp(1, 16_384)).unwrap_or(u16::MAX));
        PackOptions {
            margin: (self.margin_pixels.max(0.0) / size).min(0.25),
            rotate: self.rotate,
        }
    }
}

/// The labels of the projections UV ▸ Project offers.
pub const PROJECTIONS: [(Projection, &str); 4] = [
    (Projection::Box, key("Box")),
    (Projection::Spherical, key("Spherical")),
    (Projection::Cylindrical, key("Cylindrical")),
    (Projection::Flat, key("Flat")),
];

/// `data` (a polygon object) after `op` with `settings`; `edges_cut`: the
/// selected edges count as seams (Edges mode). `None` if it is not a polygon
/// object, has nothing to work on, or nothing changed.
#[must_use]
pub fn apply(data: &NodeData, op: UvOp, settings: &UvSettings, edges_cut: bool) -> Option<NodeData> {
    let ObjectKind::Polygon { mesh, components } = &data.kind else {
        return None;
    };
    let source = mesh.mesh();
    let topology = mesh.topology();
    let selected = components.selected();
    let hidden = &components.hidden().polygons;
    // The selected visible polygons, else every visible one.
    let any_selected = selected.polygons.iter().any(|f| !hidden.contains(f));
    let chosen: BitSet = (0..source.face_count())
        .filter(|&f| !hidden.contains(f) && (!any_selected || selected.polygons.contains(f)))
        .collect();
    if chosen.is_empty() {
        return None;
    }
    let corners: Vec<usize> = chosen.iter().flat_map(|f| source.corner_range(f)).collect();
    let current = || source.uvs().map_or_else(|| uv::box_uvs(source), <[_]>::to_vec);
    let selected_edges =
        || selection::corners_from_edges(&selection::edges_from_corners(&selected.edges, topology), topology);

    let uvs = match op {
        UvOp::Unwrap => {
            let mut cuts = selection::edges_from_corners(&components.state().seams, topology);
            if edges_cut {
                cuts.extend(selection::edges_from_corners(&selected.edges, topology).iter());
            }
            if cuts.is_empty() {
                cuts = uv::sharp_edges(source, topology, settings.seam_angle);
            }
            let options = UnwrapOptions { pack: settings.pack() };
            uv::unwrap(source, topology, &chosen, &cuts, &options)
        }
        UvOp::Relax => uv::relax(source, topology, &chosen, &settings.relax),
        UvOp::Pack => {
            let mut uvs = current();
            let mut with_uvs = source.clone();
            with_uvs.set_uvs(Some(uvs.clone())).ok()?;
            let layout = uv::layout(&with_uvs, topology);
            let mut wanted = BitSet::new();
            for f in chosen.iter() {
                wanted.insert(usize::try_from(layout.island_of[f]).ok()?);
            }
            let islands: Vec<Vec<usize>> = layout
                .island_corners(&with_uvs)
                .into_iter()
                .enumerate()
                .filter(|(i, _)| wanted.contains(*i))
                .map(|(_, corners)| corners)
                .collect();
            uv::pack(&mut uvs, &islands, &settings.pack());
            uvs
        }
        UvOp::Project(projection) => {
            if !projection.has_shape() {
                return None;
            }
            let (min, max) = source.bounds()?;
            let projected = uv::project(projection, source, uv::fit(min, max));
            let mut uvs = current();
            for &c in &corners {
                uvs[c] = *projected.get(c)?;
            }
            uvs
        }
        UvOp::Flip(axis) => {
            let mut uvs = current();
            uv::flip(&mut uvs, &corners, axis);
            uvs
        }
        UvOp::MarkSeams | UvOp::ClearSeams => {
            let edges = selected_edges();
            let mut state = components.state().clone();
            if op == UvOp::MarkSeams {
                state.seams.extend(edges.iter());
            } else if edges.is_empty() {
                state.seams.clear();
            } else {
                state.seams.remove_all(&edges);
            }
            if state.seams == components.state().seams {
                return None;
            }
            let mut out = data.clone();
            out.kind = ObjectKind::Polygon {
                mesh: mesh.clone(),
                components: Components::new(state),
            };
            return Some(out);
        }
    };
    if source.uvs() == Some(uvs.as_slice()) {
        return None;
    }
    let mut changed = source.clone();
    changed.set_uvs(Some(uvs)).ok()?;
    let mut out = data.clone();
    out.kind = ObjectKind::Polygon {
        mesh: MeshHandle::new(changed),
        components: components.clone(),
    };
    Some(out)
}

#[cfg(test)]
mod tests {
    use glam::{Vec2, Vec3};
    use grid4d_geom::primitives;
    use grid4d_scene::{ComponentState, Node};

    use super::*;

    fn polygon(mesh: grid4d_geom::PolyMesh, state: ComponentState) -> NodeData {
        Node::new(
            "Mesh",
            ObjectKind::Polygon {
                mesh: MeshHandle::new(mesh),
                components: Components::new(state),
            },
        )
        .data
    }

    fn uvs(data: &NodeData) -> Vec<Vec2> {
        let ObjectKind::Polygon { mesh, .. } = &data.kind else {
            panic!()
        };
        mesh.mesh().uvs().unwrap().to_vec()
    }

    #[test]
    fn unwrapping_a_cube_cuts_its_sharp_edges_and_keeps_its_components() {
        let mut state = ComponentState::default();
        state.selected.points.insert(3);
        let cube = polygon(primitives::cube(Vec3::splat(2.0), [1, 1, 1]), state.clone());
        let unwrapped = apply(&cube, UvOp::Unwrap, &UvSettings::default(), false).unwrap();
        let ObjectKind::Polygon { mesh, components } = &unwrapped.kind else {
            panic!()
        };
        assert_eq!(components.state(), &state);
        assert_eq!(uv::flipped_faces(mesh.mesh()), 0);
        assert_eq!(
            uv::layout(mesh.mesh(), mesh.topology()).island_count,
            6,
            "cut at every edge"
        );
        assert!(
            uvs(&unwrapped)
                .iter()
                .all(|uv| uv.x >= 0.0 && uv.x <= 1.0 && uv.y >= 0.0 && uv.y <= 1.0)
        );
    }

    #[test]
    fn tools_work_on_the_selected_polygons_and_skip_hidden_ones() {
        let plane = primitives::plane(2.0, 2.0, 2, 1);
        let before = plane.uvs().unwrap().to_vec();
        let mut state = ComponentState::default();
        state.selected.polygons.insert(1);
        let data = polygon(plane.clone(), state);
        let flipped = apply(&data, UvOp::Flip(FlipAxis::U), &UvSettings::default(), false).unwrap();
        let after = uvs(&flipped);
        assert_eq!(after[..4], before[..4], "polygon 0 untouched");
        assert_ne!(after[4..], before[4..]);
        // Everything hidden: nothing to do.
        let mut state = ComponentState::default();
        state.hidden.polygons.insert_range(0..2);
        let hidden = polygon(plane, state);
        assert!(apply(&hidden, UvOp::Relax, &UvSettings::default(), false).is_none());
    }

    #[test]
    fn seams_are_marked_from_the_selected_edges_and_cleared() {
        let cube = primitives::cube(Vec3::splat(2.0), [1, 1, 1]);
        let mut state = ComponentState::default();
        state.selected.edges.insert(0);
        let data = polygon(cube, state);
        let marked = apply(&data, UvOp::MarkSeams, &UvSettings::default(), false).unwrap();
        let ObjectKind::Polygon { components, .. } = &marked.kind else {
            panic!()
        };
        assert_eq!(components.state().seams.len(), 2, "both sides of the edge");
        assert!(
            apply(&marked, UvOp::MarkSeams, &UvSettings::default(), false).is_none(),
            "nothing new"
        );
        let cleared = apply(&marked, UvOp::ClearSeams, &UvSettings::default(), false).unwrap();
        let ObjectKind::Polygon { components, .. } = &cleared.kind else {
            panic!()
        };
        assert!(components.state().seams.is_empty());
        // Projections need a shape.
        assert!(apply(&data, UvOp::Project(Projection::Uv), &UvSettings::default(), false).is_none());
        let projected = apply(
            &data,
            UvOp::Project(Projection::Spherical),
            &UvSettings::default(),
            false,
        )
        .unwrap();
        assert_ne!(uvs(&projected), uvs(&data));
        assert_eq!(UvSettings::default().pack().margin, 4.0 / 1024.0);
    }
}
