//! The crate's edge: Grid4D's glam (centimetres) ⇄ rapier's glam (metres),
//! through plain arrays, so the two glam versions never meet.

use glam::{Quat, Vec3};
use rapier3d::prelude::{Rotation, Vector};

/// Metres per centimetre.
pub(crate) const METRES: f32 = 0.01;

/// A position or vector in centimetres, in the engine's metres.
pub(crate) fn to_engine(v: Vec3) -> Vector {
    let [x, y, z] = (v * METRES).to_array();
    Vector::new(x, y, z)
}

/// A length in centimetres, in metres.
pub(crate) fn length_to_engine(cm: f32) -> f32 {
    cm * METRES
}

/// An engine position or vector (metres), in centimetres.
pub(crate) fn from_engine(v: Vector) -> Vec3 {
    Vec3::from_array(v.to_array()) / METRES
}

/// An engine rotation as ours.
pub(crate) fn rotation_from_engine(q: Rotation) -> Quat {
    Quat::from_array(q.to_array())
}

/// A rotation of ours as the engine's.
pub(crate) fn rotation_to_engine(q: Quat) -> Rotation {
    let [x, y, z, w] = q.to_array();
    Rotation::from_xyzw(x, y, z, w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_cross_the_edge_unchanged_but_for_units() {
        let v = Vec3::new(150.0, -2.5, 1e4);
        assert!(from_engine(to_engine(v)).abs_diff_eq(v, 1e-2));
        assert_eq!(to_engine(Vec3::X * 100.0).x, 1.0);
        let q = Quat::from_rotation_y(0.7);
        assert!(rotation_from_engine(rotation_to_engine(q)).abs_diff_eq(q, 1e-6));
    }
}
