//! Any bytes as two shapes for the mesh Boolean (step 17.2): boxes, spheres
//! and cylinders, sized, turned and placed by the bytes — often on a coarse
//! grid, so faces lie in one plane, edges and points meet and shapes touch.
//! Closed shapes must give closed results in Union, Subtract and Intersect,
//! and nothing may panic. The rest of the bytes, read as a raw mesh (any
//! points and faces), must give a result or an error, never a panic.
//!
//! cargo +nightly fuzz run boolean fuzz/corpus/boolean

#![no_main]

use glam::{Quat, Vec3};
use grid4d_geom::{
    PolyMesh,
    boolean::{BooleanMode, BooleanOptions, boolean, is_closed},
    primitives,
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

    /// A value in `-range..=range`, on a grid of `step`.
    fn snapped(&mut self, step: f32, range: i32) -> f32 {
        let span = 2 * range + 1;
        (i32::from(self.byte()) % span - range) as f32 * step
    }
}

fn shape(bytes: &mut Bytes<'_>) -> PolyMesh {
    let kind = bytes.byte() % 4;
    let size = |b: &mut Bytes<'_>| f32::from(b.byte() % 8 + 1) * 0.5;
    let mut mesh = match kind {
        0 => primitives::cube(Vec3::splat(size(bytes)), [1, 1, 1]),
        1 => primitives::cube(Vec3::new(size(bytes), size(bytes), size(bytes)), [1, 1, 1]),
        2 => primitives::sphere(size(bytes) * 0.5, u32::from(bytes.byte() % 8) + 4),
        _ => primitives::cylinder(size(bytes) * 0.4, size(bytes), u32::from(bytes.byte() % 8) + 3, 1),
    };
    let turn = match bytes.byte() % 3 {
        0 => Quat::IDENTITY,
        1 => Quat::from_rotation_y(f32::from(bytes.byte() % 4) * std::f32::consts::FRAC_PI_2),
        _ => Quat::from_euler(
            glam::EulerRot::YXZ,
            f32::from(bytes.byte()) * 0.0246,
            f32::from(bytes.byte()) * 0.0246,
            f32::from(bytes.byte()) * 0.0246,
        ),
    };
    let place = if bytes.byte() % 2 == 0 {
        Vec3::new(bytes.snapped(0.25, 8), bytes.snapped(0.25, 8), bytes.snapped(0.25, 8))
    } else {
        Vec3::new(bytes.snapped(0.013, 100), bytes.snapped(0.017, 100), bytes.snapped(0.011, 100))
    };
    for p in mesh.points_mut() {
        *p = turn * *p + place;
    }
    mesh
}

/// Any points and faces from the bytes (most faces will be invalid and are
/// skipped).
fn raw(bytes: &mut Bytes<'_>) -> PolyMesh {
    let mut mesh = PolyMesh::new();
    let points = usize::from(bytes.byte() % 16) + 3;
    for _ in 0..points {
        let p = Vec3::new(bytes.snapped(0.5, 4), bytes.snapped(0.5, 4), bytes.snapped(0.5, 4));
        let _ = mesh.add_point(p);
    }
    for _ in 0..bytes.byte() % 16 {
        let corners = usize::from(bytes.byte() % 4) + 3;
        let face: Vec<u32> = (0..corners).map(|_| u32::from(bytes.byte()) % points as u32).collect();
        let _ = mesh.add_face(&face);
    }
    mesh
}

fuzz_target!(|data: &[u8]| {
    let mut bytes = Bytes(data);
    let (a, b) = (shape(&mut bytes), shape(&mut bytes));
    for mode in [BooleanMode::Union, BooleanMode::Subtract, BooleanMode::Intersect] {
        let result = boolean(&a, &b, mode, BooleanOptions::default()).expect("finite shapes");
        assert!(is_closed(&result.mesh), "{mode:?} of closed shapes is closed");
    }
    let _ = boolean(&a, &b, BooleanMode::Without, BooleanOptions::default());
    let (c, d) = (raw(&mut bytes), raw(&mut bytes));
    for mode in BooleanMode::ALL {
        let _ = boolean(&c, &d, mode, BooleanOptions::default());
        let _ = boolean(&a, &d, mode, BooleanOptions::default());
    }
});
