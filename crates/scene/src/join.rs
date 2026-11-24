//! The Join generator's settings (`docs/reference/generator-symmetry-instance-join.md`):
//! its children merged into one mesh.

/// A Join generator's settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Join {
    /// Points closer than the tolerance become one.
    pub weld: bool,
    /// The welding distance (cm, ≥ 0).
    pub tolerance: f32,
}

impl Join {
    /// The largest tolerance accepted (cm).
    pub const MAX_TOLERANCE: f32 = 100.0;
}

impl Default for Join {
    /// Welded within 0.1 cm.
    fn default() -> Self {
        Self {
            weld: true,
            tolerance: 0.1,
        }
    }
}
