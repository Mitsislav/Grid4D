//! The Reduce generator's settings (`docs/reference/generator-reduce.md`):
//! its children made lighter by collapsing edges.

/// A Reduce generator's settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reduce {
    /// The share of the input's triangles kept (0 – 1; animatable).
    pub keep: f32,
    /// Open borders keep their outline.
    pub keep_borders: bool,
    /// UV seams are kept like borders.
    pub keep_seams: bool,
    /// All children reduced as one mesh; off: each child keeps its own share.
    pub together: bool,
}

impl Default for Reduce {
    /// Half the triangles, borders and seams kept, children together.
    fn default() -> Self {
        Self {
            keep: 0.5,
            keep_borders: true,
            keep_seams: true,
            together: true,
        }
    }
}
