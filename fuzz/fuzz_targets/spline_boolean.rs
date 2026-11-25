//! Any bytes as shapes for the Spline Boolean (step 17.5): outlines of a few
//! points on a coarse grid, so edges overlap, cross at their ends, touch and
//! run back over themselves; some segments open, some degenerate. Every mode
//! must give closed outlines whose counts add up to its points, and nothing
//! may panic.
//!
//! cargo +nightly fuzz run spline_boolean fuzz/corpus/spline_boolean

#![no_main]

use glam::Vec3;
use grid4d_geom::{
    spline::{Polyline, Segment, SplinePlane},
    spline_boolean::{SplineBoolean, SplineBooleanMode, spline_boolean},
};
use libfuzzer_sys::fuzz_target;

/// Reads bytes one at a time; zeros when they run out.
struct Bytes<'a>(&'a [u8]);

impl Bytes<'_> {
    fn byte(&mut self) -> u8 {
        let Some((&first, rest)) = self.0.split_first() else {
            return 0;
        };
        self.0 = rest;
        first
    }

    /// A coordinate on a grid of quarter units, or (rarely) any bits.
    fn coordinate(&mut self) -> f32 {
        match self.byte() {
            255 => f32::from_bits(u32::from_le_bytes([self.byte(), self.byte(), self.byte(), self.byte()])),
            b => f32::from(b % 32) * 0.25,
        }
    }
}

fn shape(bytes: &mut Bytes<'_>) -> Polyline {
    let mut shape = Polyline::default();
    for _ in 0..bytes.byte() % 3 + 1 {
        let count = bytes.byte() % 7 + 1;
        for _ in 0..count {
            shape.points.push(Vec3::new(bytes.coordinate(), bytes.coordinate(), bytes.coordinate()));
        }
        shape.segments.push(Segment {
            count: u32::from(count),
            closed: bytes.byte() % 5 != 0,
        });
    }
    shape
}

fuzz_target!(|data: &[u8]| {
    let mut bytes = Bytes(data);
    let shapes: Vec<Polyline> = (0..bytes.byte() % 4 + 1).map(|_| shape(&mut bytes)).collect();
    let plane = SplinePlane::ALL[usize::from(bytes.byte() % 3)];
    for mode in SplineBooleanMode::ALL {
        let result = spline_boolean(&shapes, &SplineBoolean { mode, plane });
        let counted: usize = result.segments.iter().map(|s| s.count as usize).sum();
        assert_eq!(counted, result.points.len());
        assert!(result.segments.iter().all(|s| s.closed && s.count >= 3));
    }
});
