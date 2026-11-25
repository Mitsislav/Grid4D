//! The Boolean generator's settings (`docs/reference/generator-boolean.md`):
//! its first two children combined as solids by
//! [`grid4d_geom::boolean::boolean`].

pub use grid4d_geom::boolean::BooleanMode;
use grid4d_geom::boolean::BooleanOptions;

/// A Boolean generator's settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Boolean {
    pub mode: BooleanMode,
    /// The pieces each face was cut into merged back into polygons.
    pub merge_faces: bool,
    /// Make Editable leaves the edges along the cut selected.
    pub select_cut_edges: bool,
    /// Points of the result closer than this are welded (cm, ≥ 0).
    pub weld: f32,
    /// The edges along the cut are hard edges (step 17.8).
    pub hard_cut_edges: bool,
}

impl Boolean {
    /// The largest weld distance accepted (cm).
    pub const MAX_WELD: f32 = 100.0;

    /// The options the geometry takes.
    #[must_use]
    pub fn options(&self) -> BooleanOptions {
        BooleanOptions {
            merge_faces: self.merge_faces,
            weld: self.weld.clamp(0.0, Self::MAX_WELD),
        }
    }
}

impl Default for Boolean {
    /// Subtract, faces merged, cut edges selected, welded within 0.01 cm.
    fn default() -> Self {
        Self {
            mode: BooleanMode::Subtract,
            merge_faces: true,
            select_cut_edges: true,
            weld: 0.01,
            hard_cut_edges: false,
        }
    }
}
